//! AVI подготавливается в фоне. Одновременно декодируется только один кадр;
//! при отставании берём актуальный кадр звуковой дорожки, не накапливаем очередь.
use bevy::{
    asset::RenderAssetUsages,
    audio::{AddAudioSource, ChannelCount, SampleRate, SeekError, Source},
    image::{CompressedImageFormats, ImageSampler, ImageType},
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};
use sha2::{Digest, Sha256};
use shadow_formats::{AssetCatalog, AviMovie, PcmSamples};
use std::{
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

#[derive(Resource)]
pub(crate) struct Request {
    pub catalog: Arc<AssetCatalog>,
    pub path: String,
    pub snapshot: Option<Duration>,
}
#[derive(Component)]
pub(crate) struct Picture;
#[derive(Component)]
pub(crate) struct LoadingLabel;
#[derive(Component)]
pub(crate) struct Sound;
#[derive(Message)]
pub(crate) struct Finished(pub Option<String>);
struct Prepared {
    movie: Arc<AviMovie>,
    image: Image,
    frame: usize,
}
#[derive(Resource)]
pub(crate) struct Pending(Task<Result<Prepared, String>>);
#[derive(Asset, TypePath)]
pub(crate) struct MovieAudio {
    movie: Arc<AviMovie>,
}
pub(crate) struct Decoder {
    samples: PcmSamples,
    info: shadow_formats::AviAudio,
}
impl Iterator for Decoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        self.samples.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.samples.size_hint()
    }
}
impl Source for Decoder {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.samples.len())
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.info.channels).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.info.sample_rate).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(self.info.duration())
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.samples.seek(pos);
        Ok(())
    }
}
impl Decodable for MovieAudio {
    type Decoder = Decoder;
    fn decoder(&self) -> Decoder {
        Decoder {
            samples: self.movie.pcm().unwrap(),
            info: self.movie.audio.unwrap(),
        }
    }
}
#[derive(Default)]
struct Clock {
    position: Duration,
    wall: Duration,
    changed: Duration,
}
impl Clock {
    /// При окончании звука продолжаем видео по времени; зависший sink тоже
    /// не удерживает экран навечно. В норме положение звука задаёт кадр.
    fn advance(&mut self, wall: Duration, audio: Option<Duration>) -> Duration {
        if let Some(audio) = audio.filter(|&a| a > self.position) {
            self.position = audio;
            self.changed = wall;
        } else if audio.is_none() || wall.saturating_sub(self.changed) > Duration::from_secs(1) {
            self.position += wall.saturating_sub(self.wall);
        }
        self.wall = wall;
        self.position
    }
}
type FrameResult = (usize, Result<Image, String>);
/// JPEG не ждёт тяжёлые задачи общего пула Bevy. Обе очереди ограничены
/// одним элементом, одновременно ждём только один кадр.
struct DecodeWorker {
    requests: mpsc::SyncSender<usize>,
    output: Mutex<mpsc::Receiver<FrameResult>>,
    busy: bool,
}
impl DecodeWorker {
    fn spawn(
        decode: impl Fn(usize) -> Result<Image, String> + Send + 'static,
    ) -> Result<Self, String> {
        let (requests, input) = mpsc::sync_channel(1);
        let (results, output) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("shadow-mjpeg".into())
            .spawn(move || {
                while let Ok(frame) = input.recv() {
                    if results.send((frame, decode(frame))).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| format!("Не удалось запустить видеодекодер: {e}"))?;
        Ok(Self {
            requests,
            output: Mutex::new(output),
            busy: false,
        })
    }
    fn request(&mut self, frame: usize) -> Result<(), String> {
        if self.busy {
            return Err("Видеодекодер уже занят".into());
        }
        self.requests
            .try_send(frame)
            .map_err(|e| format!("Видеодекодер: {e}"))?;
        self.busy = true;
        Ok(())
    }
    fn poll(&mut self) -> Result<Option<FrameResult>, String> {
        match self.output.lock().unwrap().try_recv() {
            Ok(frame) => {
                self.busy = false;
                Ok(Some(frame))
            }
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err("Видеодекодер остановлен".into()),
        }
    }
}
#[derive(Resource)]
pub(crate) struct Playback {
    movie: Arc<AviMovie>,
    pub image: Handle<Image>,
    pub audio: Option<Handle<MovieAudio>>,
    prepared_at: Instant,
    started: Option<Instant>,
    clock: Clock,
    worker: DecodeWorker,
    pub frame: usize,
    pub decoded: usize,
    pub audio_started: bool,
    ended: bool,
}
impl Playback {
    pub(crate) fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"frame":self.frame,"decoded":self.decoded,"audio_started":self.audio_started,"position":self.clock.position.as_secs_f64(),"decoding":self.worker.busy,"ended":self.ended})
    }
}
pub(crate) fn install(app: &mut App) {
    app.add_audio_source::<MovieAudio>()
        .add_message::<Finished>();
}
pub(crate) fn start(mut commands: Commands, request: Res<Request>) {
    let catalog = request.catalog.clone();
    let path = request.path.clone();
    let snapshot = request.snapshot;
    commands.insert_resource(Pending(AsyncComputeTaskPool::get().spawn(async move {
        let movie = Arc::new(
            AviMovie::from_bytes(catalog.read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?,
        );
        let frame = movie.frame_at(snapshot.unwrap_or_default());
        let image = decode_frame(&movie, frame)?;
        Ok(Prepared {
            movie,
            image,
            frame,
        })
    })));
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    mut commands: Commands,
    request: Res<Request>,
    pending: Option<ResMut<Pending>>,
    playback: Option<ResMut<Playback>>,
    mut images: ResMut<Assets<Image>>,
    mut audio_assets: ResMut<Assets<MovieAudio>>,
    pictures: Query<Entity, With<Picture>>,
    labels: Query<Entity, With<LoadingLabel>>,
    sounds: Query<(Entity, Option<&AudioSink>), With<Sound>>,
    mut finished: MessageWriter<Finished>,
) {
    if let Some(mut pending) = pending {
        if let Some(result) = block_on(poll_once(&mut pending.0)) {
            commands.remove_resource::<Pending>();
            match result {
                Err(e) => {
                    finished.write(Finished(Some(e)));
                }
                Ok(prepared) => {
                    let movie = prepared.movie.clone();
                    let worker = match DecodeWorker::spawn(move |frame| decode_frame(&movie, frame))
                    {
                        Ok(worker) => worker,
                        Err(e) => {
                            finished.write(Finished(Some(e)));
                            return;
                        }
                    };
                    let image = images.add(prepared.image);
                    for entity in &pictures {
                        commands
                            .entity(entity)
                            .insert(ImageNode::new(image.clone()));
                    }
                    for entity in &labels {
                        commands.entity(entity).despawn();
                    }
                    let audio = if prepared.movie.audio.is_some() && request.snapshot.is_none() {
                        let handle = audio_assets.add(MovieAudio {
                            movie: prepared.movie.clone(),
                        });
                        commands.spawn((
                            Sound,
                            AudioPlayer(handle.clone()),
                            PlaybackSettings::ONCE.paused(),
                        ));
                        Some(handle)
                    } else {
                        None
                    };
                    commands.insert_resource(Playback {
                        movie: prepared.movie,
                        image,
                        audio,
                        prepared_at: Instant::now(),
                        started: None,
                        clock: Clock::default(),
                        worker,
                        frame: prepared.frame,
                        decoded: 1,
                        audio_started: false,
                        ended: false,
                    });
                }
            }
        }
        return;
    }
    let Some(mut p) = playback else {
        return;
    };
    if p.ended {
        return;
    }
    if p.started.is_none() {
        if let Some(sink) = sounds.iter().find_map(|(_, sink)| sink) {
            sink.play();
            p.audio_started = true;
            p.started = Some(Instant::now());
        } else if p.audio.is_none() || p.prepared_at.elapsed() >= Duration::from_secs(1) {
            // Bevy не создаёт sink без устройства. Удаляем ожидающий player,
            // чтобы звук не включился позднее в другой точке видеоряда.
            for (entity, _) in &sounds {
                commands.entity(entity).despawn();
            }
            if p.audio.is_some() {
                warn!("AVI: звук недоступен, воспроизведение без звука");
            }
            p.started = Some(Instant::now());
        }
    }
    let result = match p.worker.poll() {
        Ok(result) => result,
        Err(e) => {
            p.ended = true;
            finished.write(Finished(Some(e)));
            return;
        }
    };
    if let Some((frame, result)) = result {
        match result {
            Ok(image) => {
                images
                    .insert(p.image.id(), image)
                    .expect("живой video handle");
                p.frame = frame;
                p.decoded += 1;
            }
            Err(e) => {
                p.ended = true;
                finished.write(Finished(Some(e)));
                return;
            }
        }
    }
    let Some(start) = p.started else {
        return;
    };
    if request.snapshot.is_some() {
        return;
    }
    let sample = sounds.iter().find_map(|(_, sink)| {
        sink.filter(|s| !s.empty() && !s.is_paused())
            .map(AudioSinkPlayback::position)
    });
    let time = p.clock.advance(start.elapsed(), sample);
    let end = p
        .movie
        .duration()
        .max(p.movie.audio.map(|a| a.duration()).unwrap_or_default());
    if time >= end {
        debug!(target: "shadow_video", "end: time={time:?}, frame={}, decoded={}", p.frame, p.decoded);
        p.ended = true;
        finished.write(Finished(None));
        return;
    }
    let frame = p.movie.frame_at(time);
    debug!(target: "shadow_video", "time={time:?}, requested={frame}, displayed={}, decoding={}", p.frame, p.worker.busy);
    if frame != p.frame
        && !p.worker.busy
        && let Err(e) = p.worker.request(frame)
    {
        p.ended = true;
        finished.write(Finished(Some(e)));
    }
}
pub(crate) fn fit(
    window: Single<&Window>,
    playback: Option<Res<Playback>>,
    mut pictures: Query<&mut Node, With<Picture>>,
) {
    let Some(p) = playback else {
        return;
    };
    let (origin, size) = contain(
        [window.width(), window.height()],
        [p.movie.width as f32, p.movie.height as f32],
    );
    for mut node in &mut pictures {
        node.left = px(origin[0]);
        node.top = px(origin[1]);
        node.width = px(size[0]);
        node.height = px(size[1]);
    }
}
fn contain(window: [f32; 2], source: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    let scale = (window[0] / source[0]).min(window[1] / source[1]);
    let size = [source[0] * scale, source[1] * scale];
    (
        [(window[0] - size[0]) * 0.5, (window[1] - size[1]) * 0.5],
        size,
    )
}
pub(crate) fn cleanup(
    mut commands: Commands,
    playback: Option<Res<Playback>>,
    sounds: Query<(Entity, Option<&AudioSink>), With<Sound>>,
    mut images: ResMut<Assets<Image>>,
    mut audio: ResMut<Assets<MovieAudio>>,
) {
    for (entity, sink) in &sounds {
        if let Some(sink) = sink {
            sink.stop();
        }
        commands.entity(entity).despawn();
    }
    if let Some(p) = playback {
        images.remove(p.image.id());
        if let Some(handle) = &p.audio {
            audio.remove(handle.id());
        }
    }
    commands.remove_resource::<Pending>();
    commands.remove_resource::<Playback>();
    commands.remove_resource::<Request>();
}
fn jpeg_size(bytes: &[u8]) -> Result<[u32; 2], String> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err("MJPEG: нет JPEG SOI".into());
    }
    let mut at = 2;
    while at < bytes.len() {
        if bytes[at] != 0xff {
            return Err("MJPEG: неверный маркер JPEG".into());
        }
        while bytes.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = *bytes.get(at).ok_or("MJPEG: обрезан маркер JPEG")?;
        at += 1;
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            continue;
        }
        let length = bytes.get(at..at + 2).ok_or("MJPEG: обрезан сегмент JPEG")?;
        let length = usize::from(u16::from_be_bytes(length.try_into().unwrap()));
        if length < 2 || at + length > bytes.len() {
            return Err("MJPEG: сегмент вне файла".into());
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if length < 8 {
                return Err("MJPEG: короткий SOF".into());
            }
            return Ok([
                u32::from(u16::from_be_bytes([bytes[at + 5], bytes[at + 6]])),
                u32::from(u16::from_be_bytes([bytes[at + 3], bytes[at + 4]])),
            ]);
        }
        at += length;
    }
    Err("MJPEG: нет размеров JPEG".into())
}
fn decode_frame(movie: &AviMovie, frame: usize) -> Result<Image, String> {
    let bytes = movie.frame(frame).ok_or("AVI: неверный номер кадра")?;
    if jpeg_size(bytes)? != [movie.width, movie.height] {
        return Err("MJPEG: размеры JPEG не совпадают с AVI".into());
    }
    Image::from_buffer(
        bytes,
        ImageType::Extension("jpg"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::linear(),
        RenderAssetUsages::default(),
    )
    .map_err(|e| e.to_string())
}
pub(crate) fn write_report(
    catalog: &AssetCatalog,
    path: &str,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let movie = Arc::new(AviMovie::from_bytes(catalog.read(path)?)?);
    let mut frames = Vec::new();
    for index in [0, movie.frame_count() / 2, movie.frame_count() - 1] {
        let image = decode_frame(&movie, index)?;
        let file = out.with_extension(format!("frame-{index}.png"));
        let rgba = image.try_into_dynamic()?.to_rgba8();
        let hash = format!("{:x}", Sha256::digest(rgba.as_raw()));
        rgba.save(&file)?;
        frames.push(serde_json::json!({"index":index,"jpeg_sha256":format!("{:x}", Sha256::digest(movie.frame(index).unwrap())),"rgba_sha256":hash,"png":file}));
    }
    let audio = movie.audio.map(|a| {
        let mut hash = Sha256::new();
        for sample in movie.pcm().unwrap() { hash.update(((sample * 32768.0) as i16).to_le_bytes()); }
        serde_json::json!({"codec":"PCM16LE","sample_rate":a.sample_rate,"channels":a.channels,"samples":a.samples,"duration":a.duration().as_secs_f64(),"pcm_sha256":format!("{:x}",hash.finalize())})
    });
    let report = serde_json::json!({"path":path,"source":catalog.source_label(path)?,"video_codec":"MJPG","size":[movie.width,movie.height],"scale":movie.scale,"rate":movie.rate,"frames":movie.frame_count(),"duration":movie.duration().as_secs_f64(),"audio":audio,"decoded_frames":frames});
    std::fs::write(out, serde_json::to_string_pretty(&report)? + "\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoder_worker_bounds_requests_delivers_and_exits_on_drop() {
        struct Exit(mpsc::SyncSender<()>);
        impl Drop for Exit {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (exited, wait) = mpsc::sync_channel(1);
        let guard = Exit(exited);
        let mut worker = DecodeWorker::spawn(move |_| {
            let _keep_alive = &guard;
            Ok(Image::default())
        })
        .unwrap();
        worker.request(7).unwrap();
        assert!(worker.request(8).is_err());
        let (frame, image) = worker
            .output
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(frame, 7);
        assert!(image.is_ok());
        drop(worker);
        wait.recv_timeout(Duration::from_secs(5))
            .expect("decoder thread exited and dropped its captures");
    }
    #[test]
    fn clock_follows_audio_and_continues_after_audio_end() {
        let mut c = Clock::default();
        assert_eq!(
            c.advance(Duration::from_millis(50), Some(Duration::from_millis(40))),
            Duration::from_millis(40)
        );
        assert_eq!(
            c.advance(Duration::from_millis(80), Some(Duration::from_millis(70))),
            Duration::from_millis(70)
        );
        assert_eq!(
            c.advance(Duration::from_millis(100), None),
            Duration::from_millis(90)
        );
        assert_eq!(
            c.advance(Duration::from_millis(120), None),
            Duration::from_millis(110)
        );
    }
    #[test]
    fn clock_without_audio_and_stalled_device_progresses() {
        let mut c = Clock::default();
        assert_eq!(
            c.advance(Duration::from_secs(1), None),
            Duration::from_secs(1)
        );
        let mut c = Clock::default();
        c.advance(Duration::from_millis(500), Some(Duration::ZERO));
        assert_eq!(
            c.advance(Duration::from_millis(1500), Some(Duration::ZERO)),
            Duration::from_secs(1)
        );
        assert_eq!(
            c.advance(Duration::from_millis(1700), Some(Duration::ZERO)),
            Duration::from_millis(1200)
        );
    }
    #[test]
    fn movies_keep_source_proportions_in_modern_windows() {
        for window in [[1280.0, 720.0], [1920.0, 1080.0], [3440.0, 1440.0]] {
            for source in [[640.0, 480.0], [854.0, 480.0], [3840.0, 2160.0]] {
                let (origin, size) = contain(window, source);
                assert!(origin.iter().all(|&v| v >= -0.001));
                assert!((size[0] / size[1] - source[0] / source[1]).abs() < 0.001);
                assert!((origin[0] * 2.0 + size[0] - window[0]).abs() < 0.01);
                assert!((origin[1] * 2.0 + size[1] - window[1]).abs() < 0.01);
            }
        }
    }
    #[test]
    fn jpeg_header_checks_truncation_and_dimensions_before_decode() {
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0, 3, 0, 0xff, 0xc0, 0, 8, 8, 1, 0xe0, 2, 0x80, 3,
        ];
        assert_eq!(jpeg_size(&jpeg).unwrap(), [640, 480]);
        for end in 0..jpeg.len() {
            assert!(jpeg_size(&jpeg[..end]).is_err());
        }
    }
}
