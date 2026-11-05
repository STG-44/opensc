# Current map scale limits

The terrain reader currently accepts 2–2048 height samples per axis and at most
1,048,576 samples in total. A square grid can therefore reach 1024 × 1024; a
2048 × 512 grid also fits. These are input validation limits, not a performance
guarantee. Physical width is `sample_count × nominal_edge_size × 0.01` metres.

The renderer builds the whole terrain mesh and loads the scene at once. Object
visibility, navigation, radar exploration and collision also keep scene-wide
state. Very large maps require streamed sectors, terrain/object levels of
detail, partitioned gameplay data and an explicit coordinate precision policy.
Simply raising the reader's bounds does not provide those features.

Original maps keep their original terrain and object coordinates. No sector
streaming or unlimited-world claim is made by the current engine.
