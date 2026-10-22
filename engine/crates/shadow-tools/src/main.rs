use serde_json::json;
use sha2::{Digest, Sha256};
use shadow_formats::{AssetCatalog, GutArchive};
use std::{env, error::Error, fs::OpenOptions, io::Write, path::Path};
mod maps;
mod missions;
mod models;
mod motions;

fn main() {
    if let Err(error) = run() {
        eprintln!("Ошибка: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|arg| arg == "--help") {
        println!(
            "shadow-tools summary|list|extract --game-dir PATH [--overrides PATH] [--filter TEXT] [--asset NAME --output FILE]\nshadow-tools archive --file FILE.gut --json [--hash]\nshadow-tools motion|motions --game-dir PATH [--asset NAME.bmf] --json\nshadow-tools map|maps|heightmap --game-dir PATH [--asset NAME] --json\nshadow-tools mission --game-dir PATH [--overrides PATH] --asset campaign/NAME/NAME.txt --json\nshadow-tools model|models --game-dir PATH [--overrides PATH] [--asset NAME.smf] [--filter TEXT] --json"
        );
        return Ok(());
    }
    if !matches!(
        args[0].as_str(),
        "summary"
            | "list"
            | "extract"
            | "archive"
            | "model"
            | "models"
            | "motion"
            | "motions"
            | "map"
            | "maps"
            | "heightmap"
            | "mission"
    ) {
        return Err(format!("неизвестная команда {}; используй --help", args[0]).into());
    }
    let mut index = 1;
    let mut seen = std::collections::HashSet::new();
    while index < args.len() {
        let option = args[index].as_str();
        if !seen.insert(option) {
            return Err(format!("повторяющийся аргумент {option}").into());
        }
        if (option == "--hash" && args[0] == "archive")
            || (option == "--json"
                && matches!(
                    args[0].as_str(),
                    "archive"
                        | "model"
                        | "models"
                        | "motion"
                        | "motions"
                        | "map"
                        | "maps"
                        | "heightmap"
                        | "mission"
                ))
        {
            index += 1;
            continue;
        }
        if !matches!(
            option,
            "--game-dir" | "--overrides" | "--filter" | "--asset" | "--output" | "--file"
        ) {
            return Err(format!("неизвестный аргумент {option}").into());
        }
        if args
            .get(index + 1)
            .is_none_or(|value| value.starts_with("--"))
        {
            return Err(format!("после {option} нужно значение").into());
        }
        index += 2;
    }
    let flag = |name: &str| -> Option<&str> {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|index| args.get(index + 1))
            .map(String::as_str)
    };
    let required = |name: &str| flag(name).ok_or_else(|| format!("не указан {name}"));
    if args[0] == "archive" {
        let archive = GutArchive::open(required("--file")?)?;
        let hash = args.iter().any(|arg| arg == "--hash");
        let mut entries = Vec::new();
        for (index, entry) in archive.entries.iter().enumerate() {
            let mut value = json!({"name": entry.name, "size": entry.size, "offset": entry.offset,
                "encoded": entry.encoded, "stored_checksum": entry.stored_checksum});
            if hash {
                value["sha256"] =
                    json!(format!("{:x}", Sha256::digest(archive.read_entry(index)?)));
            }
            entries.push(value);
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"name":archive.name,"entries":entries}))?
        );
        return Ok(());
    }
    let game_dir = required("--game-dir")?;
    let catalog = AssetCatalog::open(game_dir, flag("--overrides").map(Path::new))?;
    match args[0].as_str() {
        "mission" => println!(
            "{}",
            serde_json::to_string_pretty(&missions::describe(&catalog, required("--asset")?)?)?
        ),
        "model" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&models::describe(&catalog, required("--asset")?)?)?
            );
        }
        "models" => {
            let filter = flag("--filter").unwrap_or("").to_lowercase();
            let models = catalog
                .paths()
                .filter(|path| path.ends_with(".smf") && path.contains(&filter))
                .map(|path| models::describe(&catalog, path))
                .collect::<Result<Vec<_>, _>>()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"models": models}))?
            );
        }
        "motion" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&motions::describe(&catalog, required("--asset")?)?)?
            );
        }
        "motions" => {
            let filter = flag("--filter").unwrap_or("").to_lowercase();
            let motions = catalog
                .paths()
                .filter(|p| p.ends_with(".bmf") && p.contains(&filter))
                .map(|p| motions::describe(&catalog, p))
                .collect::<Result<Vec<_>, _>>()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"motions":motions}))?
            );
        }
        "map" => println!(
            "{}",
            serde_json::to_string_pretty(&maps::describe(&catalog, required("--asset")?)?)?
        ),
        "maps" => {
            let values = catalog
                .paths()
                .filter(|p| p.ends_with(".mtf"))
                .map(|p| maps::describe(&catalog, p))
                .collect::<Result<Vec<_>, _>>()?;
            println!("{}", serde_json::to_string_pretty(&json!({"maps":values}))?);
        }
        "heightmap" => println!(
            "{}",
            serde_json::to_string_pretty(&maps::heightmap(&catalog, required("--asset")?)?)?
        ),
        "summary" => {
            for archive in &catalog.archives {
                println!("{:<18} {:>5} записей", archive.name, archive.entries.len());
            }
            println!(
                "Всего архивов: {}\nРесурсов с учётом замен: {}",
                catalog.archives.len(),
                catalog.paths().count()
            );
        }
        "list" => {
            let filter = flag("--filter").unwrap_or("").to_lowercase();
            for path in catalog.paths().filter(|path| path.contains(&filter)) {
                println!("{path}\t{}", catalog.source_label(path)?);
            }
        }
        "extract" => {
            let asset = required("--asset")?;
            let output = required("--output")?;
            let bytes = catalog.read(asset)?;
            // create_new предотвращает случайную перезапись существующего файла.
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output)?;
            file.write_all(&bytes)?;
            println!("{} байт → {output}", bytes.len());
        }
        command => return Err(format!("неизвестная команда {command}").into()),
    }
    Ok(())
}
