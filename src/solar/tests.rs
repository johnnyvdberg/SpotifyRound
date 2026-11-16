use super::*;

fn read_texture(gpu: &Graphics, texture: &wgpu::Texture) -> Vec<u8> {
    let row = SIZE * 4;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: padded as u64 * SIZE as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
    let data = slice.get_mapped_range().expect("mapped");
    let mut out = Vec::with_capacity((row * SIZE) as usize);
    for y in 0..SIZE as usize {
        out.extend_from_slice(&data[y * padded as usize..][..row as usize]);
    }
    out
}

/// Run the streamer until everything wanted at `t` is in.
fn settle(solar: &mut Solar, gpu: &Graphics, t: f64) {
    for _ in 0..100_000 {
        // Asked again before looking: a planet finished in the last draw
        // leaves the streamer idle until the next one wanted is started.
        solar.stream(t);
        let baked = solar.starfield_slot(solar.tour.system.seed).is_some();
        if !solar.streamer.busy() && baked {
            break;
        }
        solar.draw(gpu, t, 0);
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
    }
}

/// The moments of one visit worth a look: the start, middle and end of
/// every crossing, each orbit high and low and the dive and climb between,
/// the departure, and the jump as it spools up, at full speed and as it
/// drops out.
fn moments(tour: &Tour) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for segment in &tour.segments {
        let at = |u: f64| segment.start + segment.duration * u;
        let end = segment.start + segment.duration;
        match segment.phase {
            Phase::Transfer { from, to } => {
                let from = from.map_or("dropout".to_string(), |p| p.to_string());
                out.push((
                    format!("transfer-{from}-to-{to}-start"),
                    segment.start + 0.5,
                ));
                out.push((format!("transfer-{from}-to-{to}-departure-burn"), at(0.06)));
                match tour.flyby_at(to) {
                    Some(closest) => {
                        out.push((format!("transfer-{from}-to-{to}-flyby-near"), closest - 3.0));
                        out.push((format!("transfer-{from}-to-{to}-flyby"), closest));
                    }
                    None => out.push((format!("transfer-{from}-to-{to}-cruise"), at(0.5))),
                }
                out.push((format!("transfer-{from}-to-{to}-capture-burn"), at(0.94)));
                out.push((format!("transfer-{from}-to-{to}-arrival"), end - 1.5));
            }
            Phase::High(p) => out.push((format!("{p}-high-orbit"), at(0.5))),
            Phase::Base(_, stage) => {
                let name = stage.name();
                if stage == tour::BaseStage::Approach {
                    // Each step of the way in.
                    let base = tour.base.as_ref().expect("a starbase");
                    let steps = base.steps();
                    for (i, &(step, start)) in steps.iter().enumerate() {
                        let next = steps.get(i + 1).map_or(segment.duration, |s| s.1);
                        out.push((
                            format!("base-{name}-{}", format!("{step:?}").to_lowercase()),
                            segment.start + (start + next) / 2.0,
                        ));
                    }
                } else {
                    // Each stage at the moments worth a look: the funnel and
                    // the tunnel's windows, the clamps closing, the service
                    // through its steps, the lift out of the cradle, the crossing
                    // of the world, the far lock and out of the far mouth.
                    let points: &[(&str, f64)] = match stage {
                        tour::BaseStage::Airlock => {
                            &[("entrance", 0.2), ("sealed", 0.7), ("opening", 0.9)]
                        }
                        tour::BaseStage::Tunnel => &[
                            ("funnel", 0.08),
                            ("windows", 0.45),
                            ("bend", 0.75),
                            ("bay-ahead", 0.97),
                        ],
                        tour::BaseStage::Berth => &[("hall", 0.3), ("clamps", 0.93)],
                        tour::BaseStage::Service => &[
                            ("weld", 0.1),
                            ("couple", 0.3),
                            ("fuel", 0.5),
                            ("scan", 0.7),
                            ("ready", 0.95),
                        ],
                        tour::BaseStage::Undock => &[("lift", 0.3), ("on", 0.9)],
                        tour::BaseStage::Crossing => &[
                            ("port", 0.02),
                            ("bulkheads", 0.045),
                            ("speeding", 0.1),
                            ("cruise", 0.3),
                            ("deep", 0.62),
                            ("slowing", 0.93),
                            ("wall", 0.97),
                        ],
                        tour::BaseStage::Egress => &[
                            ("lock", 0.2),
                            ("sealed", 0.39),
                            ("opening", 0.5),
                            ("entrance", 0.75),
                            ("mouth", 0.97),
                        ],
                        _ => &[("out", 0.15), ("orbit", 0.8)],
                    };
                    for &(what, u) in points {
                        out.push((format!("base-{name}-{what}"), at(u)));
                    }
                }
            }
            Phase::Depart(_) => {
                out.push(("depart-start".into(), segment.start + 1.0));
                out.push(("depart-spool".into(), end - 1.0));
            }
            Phase::Cruise { .. } => {
                out.push(("cruise-start".into(), segment.start + 1.0));
                out.push(("cruise".into(), at(0.5)));
                out.push(("cruise-arrival".into(), end - 1.0));
            }
            Phase::Spool => {
                for (i, &start) in tour::SPOOL_STAGES.iter().enumerate() {
                    out.push((format!("spool-stage{i}"), at(start + 0.04)));
                }
                out.push(("spool-engage".into(), end - 0.5));
            }
            Phase::Jump(k) => {
                // The last leg keeps the names the previews ask for.
                let name = if k as usize + 1 == tour.passage.legs.len() {
                    "jump".to_string()
                } else {
                    format!("jump{k}")
                };
                out.push((format!("{name}-entry"), at(0.04)));
                out.push((format!("{name}-cruise"), at(0.5)));
                if tour.passage.legs[k as usize].flux.is_some() {
                    out.push((format!("{name}-flux"), at(0.55)));
                }
                out.push((format!("{name}-dropout"), end - 2.0));
            }
            Phase::Stopover(k) => {
                let slug = tour.passage.stops[k as usize].event.slug();
                out.push((format!("{slug}-dropout"), segment.start + 0.8));
                for (i, u) in [0.15, 0.4, 0.6, 0.8, 0.97].into_iter().enumerate() {
                    out.push((format!("{slug}-{i}"), at(u)));
                }
            }
            Phase::Rejump(k) => {
                let slug = tour.passage.stops[k as usize].event.slug();
                out.push((format!("{slug}-rejump"), at(0.5)));
                out.push((format!("{slug}-rejump-engage"), end - 0.4));
            }
        }
    }
    out
}

/// Some trips hop through relays, never more than two in a row, and
/// every tour's route ends at the next real visit.
#[test]
fn some_jumps_hop_through_relays() {
    let base = 7;
    let relays: Vec<bool> = (0..200).map(|v| relay(base, v)).collect();
    assert!(!relays[0]);
    let count = relays.iter().filter(|&&r| r).count();
    assert!(count > 20 && count < 100, "{count}");
    assert!(relays.windows(3).all(|w| !w.iter().all(|&r| r)));
    assert!(
        relays.windows(2).any(|w| w[0] && w[1]),
        "a trip of three jumps now and then"
    );
    for visit in 0..60 {
        let tour = tour_for(base, visit, true, None);
        assert_eq!(tour.transit, relays[visit as usize]);
        let hops = tour.route.len();
        assert!(
            relays[visit as usize + 1..visit as usize + hops]
                .iter()
                .all(|&r| r)
        );
        assert!(!relays[visit as usize + hops]);
        assert!(
            tour.leg >= 1 && tour.leg + hops - 1 == tour.legs,
            "{visit}: {} {hops} {}",
            tour.leg,
            tour.legs
        );
    }
}

/// The index buffer's grids lie where `grid_range` says, and a mesh's
/// grid follows the disc's size: fine in orbit, coarse far off.
#[test]
fn grids() {
    let indices = grid_indices();
    assert_eq!(grid_range(8), 0..384);
    assert_eq!(grid_range(96), 32640..87936);
    assert_eq!(grid_range(128), 87936..186240);
    assert_eq!(indices.len(), 186240);
    assert_eq!(indices[87936..].iter().max(), Some(&(128 * 129 + 128)));
    let pixel = (2.0 * 0.5 / f64::from(SIZE)) as f32;
    let body = |distance: f32| Body {
        place: [0.0, 0.0, distance, 1.0],
        rot: [0, 1, 2].map(|k| [0, 1, 2, 3].map(|i| f32::from(u8::from(i == k)))),
        ..Default::default()
    };
    // From 2.5 radii, a limb 470 px from the centre: cells under 1.7°, a
    // cap of 66 degrees round the side facing the camera (-z).
    let (window, mesh) = globe_mesh(&body(2.5), pixel);
    assert_eq!(mesh[0], 96.0);
    assert!(((window[0] + window[2] / 2.0).abs() - std::f32::consts::PI).abs() < 1e-4);
    assert!(window[3] > 2.0 * 66.4f32.to_radians() && window[3] < 2.0 * 75f32.to_radians());
    // Far off, a few pixels: the coarsest, every longitude.
    let (window, mesh) = globe_mesh(&body(2000.0), pixel);
    assert_eq!(mesh[0], 8.0);
    assert_eq!(window[2], std::f32::consts::TAU);
}

/// Screenshots at the critical moments of a visit, for each seed in
/// `SOLAR_SHOTS_SEEDS` (comma separated, default 0,1,2,5: between them
/// every kind of planet and moon is visited), written to
/// `SOLAR_SHOTS_DIR` (default `target/solar-shots`): one PNG per moment,
/// a contact sheet per seed, and the HUD at each moment in `hud.txt`.
/// `SOLAR_SHOTS=low` keeps only the moments whose names contain `low`.
/// The streamer is run until the moment's maps are in, so each shot shows
/// the finished assets.
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn screenshots() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    let seeds: Vec<u64> = std::env::var("SOLAR_SHOTS_SEEDS")
        .unwrap_or_else(|_| "0,1,2,5".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let only = std::env::var("SOLAR_SHOTS").ok();
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let mut notes = String::new();
    for seed in seeds {
        let mut solar = Solar::with_seed(&gpu, seed);
        let system = &solar.tour.system;
        notes.push_str(&format!(
            "seed {seed}: {} ({}, {:.0} K), itinerary {:?}\n",
            system.star.name,
            system.star.title(),
            system.star.temperature,
            solar.tour.itinerary
        ));
        for (i, planet) in system.planets.iter().enumerate() {
            notes.push_str(&format!(
                "  p{i} {} {:?} r={:.2} moons=[{}]{}\n",
                planet.name,
                planet.kind,
                planet.radius,
                planet
                    .moons
                    .iter()
                    .map(|m| format!("{} {:?}", m.name, m.kind))
                    .collect::<Vec<_>>()
                    .join(", "),
                if planet.rings.is_some() { " rings" } else { "" }
            ));
        }
        let mut thumbs = Vec::new();
        for (k, (label, t)) in moments(&solar.tour).into_iter().enumerate() {
            if only.as_deref().is_some_and(|o| !label.contains(o)) {
                continue;
            }
            settle(&mut solar, &gpu, t);
            solar.draw(&gpu, t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            let name = format!("seed{seed}-{k:02}-{label}");
            let path = std::path::Path::new(&dir).join(format!("{name}.png"));
            image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                .expect("a screenshot");
            solar.hud_at = f64::NEG_INFINITY;
            solar.update_hud(t);
            notes.push_str(&format!("  {name} (t={t:.1}s) {}\n", solar.hud.title));
            for (l, v) in &solar.hud.lines {
                notes.push_str(&format!("      {l:>13}  {v}\n"));
            }
            if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                thumbs.push(image::imageops::thumbnail(&image, 270, 270));
            }
        }
        // A contact sheet, six across.
        let across = 6u32;
        let rows = (thumbs.len() as u32).div_ceil(across);
        let mut sheet = image::RgbaImage::new(270 * across, 270 * rows.max(1));
        for (i, thumb) in thumbs.iter().enumerate() {
            let (x, y) = ((i as u32 % across) * 270, (i as u32 / across) * 270);
            image::imageops::overlay(&mut sheet, thumb, x as i64, y as i64);
        }
        sheet
            .save(std::path::Path::new(&dir).join(format!("seed{seed}-sheet.png")))
            .expect("a contact sheet");
    }
    std::fs::write(std::path::Path::new(&dir).join("hud.txt"), notes).expect("the HUD notes");
}

/// Every hyperjump drive at a few moments of a jump, to
/// `SOLAR_SHOTS_DIR` (default `target/solar-shots`): `warp-<drive>-<n>.png`
/// and a sheet of them all, a row per drive.
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn warps() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let mut solar = Solar::with_seed(&gpu, 11);
    let start = solar.tour.total - tour::JUMP;
    // The system's starfield baked, as it is long before any jump.
    settle(&mut solar, &gpu, start);
    let moments = [0.03, 0.12, 0.5, 0.85];
    let mut sheet =
        image::RgbaImage::new(270 * moments.len() as u32, 270 * tour::DRIVES.len() as u32);
    for (k, name) in tour::DRIVES.iter().enumerate() {
        solar.drive_override = Some(k as u32 + 1);
        for (m, at) in moments.iter().enumerate() {
            let t = start + tour::JUMP * at;
            solar.draw(&gpu, t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            let file = name.to_lowercase().replace(' ', "-");
            let path = std::path::Path::new(&dir).join(format!("warp-{file}-{m}.png"));
            image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                .expect("a screenshot");
            if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                let thumb = image::imageops::thumbnail(&image, 270, 270);
                image::imageops::overlay(&mut sheet, &thumb, 270 * m as i64, 270 * k as i64);
            }
        }
    }
    sheet
        .save(std::path::Path::new(&dir).join("warps-sheet.png"))
        .expect("a sheet");
}

/// Every drive starting up at the jump point, a frame in each stage of
/// its preparation and two into the jump, to `SOLAR_SHOTS_DIR` (default
/// `target/solar-shots`): `spool-<drive>-<n>.png` and `spools-sheet.png`,
/// a row per drive.
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn spools() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let mut solar = Solar::with_seed(&gpu, 11);
    let start = solar.tour.total - tour::JUMP - tour::SPOOL;
    settle(&mut solar, &gpu, start);
    // Seconds into the spool: a little past each stage's start, the
    // engagement's end, and into the jump.
    let moments: Vec<f64> = tour::SPOOL_STAGES
        .iter()
        .map(|s| tour::SPOOL * (s + 0.12))
        .chain([tour::SPOOL - 0.3, tour::SPOOL + 2.0, tour::SPOOL + 6.0])
        .collect();
    let cell = 216;
    let mut sheet = image::RgbaImage::new(
        cell * moments.len() as u32,
        cell * tour::DRIVES.len() as u32,
    );
    for (k, name) in tour::DRIVES.iter().enumerate() {
        solar.drive_override = Some(k as u32 + 1);
        for (m, at) in moments.iter().enumerate() {
            solar.draw(&gpu, start + at, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            let file = name.to_lowercase().replace(' ', "-");
            let path = std::path::Path::new(&dir).join(format!("spool-{file}-{m}.png"));
            image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                .expect("a screenshot");
            if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                let thumb = image::imageops::thumbnail(&image, cell, cell);
                image::imageops::overlay(
                    &mut sheet,
                    &thumb,
                    (cell * m as u32) as i64,
                    (cell * k as u32) as i64,
                );
            }
        }
    }
    sheet
        .save(std::path::Path::new(&dir).join("spools-sheet.png"))
        .expect("a sheet");
}

/// A relay's crossing: dropping out at its edge, across it to the jump
/// point and the drive starting up there, as `relay-<n>.png` and
/// `relay-sheet.png` in `SOLAR_SHOTS_DIR`.
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn relay_crossing() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let mut solar = Solar::with_seed(&gpu, 11);
    let (system, next) = (solar.tour.next.clone(), system_for(11, 2));
    let passage = voyage::Passage::plan(11, 2, tour::drive_to(&next).0);
    solar.tour = Tour::plan(
        system,
        vec![next.clone(), system_for(11, 3)],
        true,
        1,
        2,
        passage,
        1,
    );
    settle(&mut solar, &gpu, 0.0);
    let moments = [0.5, 1.5, 3.0, 20.0, 40.0, 60.0, 78.0, 84.0, 100.0, 112.0];
    let cell = 216;
    let mut sheet = image::RgbaImage::new(cell * moments.len() as u32, cell);
    for (m, &t) in moments.iter().enumerate() {
        solar.draw(&gpu, t, 0);
        let pixels = read_texture(&gpu, &solar.frames[0].texture);
        let path = std::path::Path::new(&dir).join(format!("relay-{m}.png"));
        image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
            .expect("a screenshot");
        if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
            let thumb = image::imageops::thumbnail(&image, cell, cell);
            image::imageops::overlay(&mut sheet, &thumb, (cell * m as u32) as i64, 0);
        }
    }
    sheet
        .save(std::path::Path::new(&dir).join("relay-sheet.png"))
        .expect("a sheet");
}

/// The drop-out from each drive into the next system: the jump's last
/// second, then the next visit's first seconds, as `dropout-<drive>-<n>.png`
/// and `dropouts-sheet.png` (a row a drive) in `SOLAR_SHOTS_DIR`.
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn drop_outs() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    // Seconds before the end of the jump (negative), then into the next visit.
    let moments = [-1.0, -0.4, -0.02, 0.0, 0.25, 0.5, 0.8, 1.2, 1.7, 2.6];
    let cell = 216;
    let mut sheet = image::RgbaImage::new(
        cell * moments.len() as u32,
        cell * tour::DRIVES.len() as u32,
    );
    for (k, name) in tour::DRIVES.iter().enumerate() {
        let mut solar = Solar::with_seed(&gpu, 11);
        solar.drive_override = Some(k as u32 + 1);
        let total = solar.tour.total;
        // The next system's starfield and first stop, as the jump has
        // them ready by its end.
        settle(&mut solar, &gpu, total - 1.0);
        let mut arrived = false;
        for (m, &at) in moments.iter().enumerate() {
            let t = if at < 0.0 {
                total + at
            } else {
                if !arrived {
                    solar.catch_up(total);
                    settle(&mut solar, &gpu, 0.0);
                    arrived = true;
                }
                at
            };
            solar.draw(&gpu, t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            let file = name.to_lowercase().replace(' ', "-");
            let path = std::path::Path::new(&dir).join(format!("dropout-{file}-{m}.png"));
            image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                .expect("a screenshot");
            if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                let thumb = image::imageops::thumbnail(&image, cell, cell);
                let (x, y) = (cell * m as u32, cell * k as u32);
                image::imageops::overlay(&mut sheet, &thumb, x as i64, y as i64);
            }
        }
    }
    sheet
        .save(std::path::Path::new(&dir).join("dropouts-sheet.png"))
        .expect("a sheet");
}

/// Wait for the starship meshes (built, or loaded from the cache) and
/// put them on the GPU.
fn wait_for_models(solar: &mut Solar, gpu: &Graphics) {
    if let Some(rx) = solar.model_library.take() {
        let library = rx.recv().expect("the ship meshes");
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(library).expect("sent");
        solar.model_library = Some(rx);
        solar.receive_models(gpu);
    }
}

/// Visit 0 of the tour seeded `base`, its jump through `stops` alone.
fn tour_through(base: u64, stops: Vec<voyage::Stopover>) -> Tour {
    let system = system_for(base, 0);
    let next = system_for(base, 1);
    let drive = tour::drive_to(&next).0;
    let mut rng = system::Rng::new(base ^ 0x5eed);
    let passage = voyage::Passage::with_stops(stops, drive, &mut rng);
    let arrival = tour::drive_to(&system).0;
    Tour::plan(system, vec![next], false, 1, 1, passage, arrival)
}

/// Saves frame `pixels` as `name` in `dir` and adds it to `sheet` at
/// column `x`, row `y`, `cell` pixels a side.
fn shoot(
    dir: &str,
    name: &str,
    pixels: Vec<u8>,
    sheet: &mut image::RgbaImage,
    cell: u32,
    x: u32,
    y: u32,
) {
    let path = std::path::Path::new(dir).join(format!("{name}.png"));
    image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
        .expect("a screenshot");
    if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
        let thumb = image::imageops::thumbnail(&image, cell, cell);
        image::imageops::overlay(sheet, &thumb, (cell * x) as i64, (cell * y) as i64);
    }
}

/// A starbase visit from the approach to the climb out, at every moment
/// `moments` names for it: a system whose ship holds for priority traffic
/// and one whose does not (the first seeds with each, or `SOLAR_SEED` for
/// one). A row each on `starbase-sheet.png`; the HUD and panels in
/// `starbase.txt`.
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn starbase() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let with_base = |hold: bool| {
        (0..600u64).find(|&seed| {
            let tour = Tour::new(
                system::System::generate(seed),
                system::System::generate(seed + 1),
            );
            tour.base
                .as_ref()
                .is_some_and(|b| b.traffic.is_some() == hold)
        })
    };
    let seeds: Vec<u64> = match std::env::var("SOLAR_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        Some(seed) => vec![seed],
        None => [with_base(true), with_base(false)]
            .into_iter()
            .flatten()
            .collect(),
    };
    let cell = 216;
    let mut rows = Vec::new();
    let mut notes = String::new();
    for seed in seeds {
        let mut solar = Solar::with_seed(&gpu, seed);
        wait_for_models(&mut solar, &gpu);
        let mut thumbs = Vec::new();
        for (label, t) in moments(&solar.tour)
            .into_iter()
            .filter(|(l, _)| l.starts_with("base-"))
        {
            settle(&mut solar, &gpu, t);
            solar.draw(&gpu, t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            let name = format!("starbase-seed{seed}-{label}");
            let path = std::path::Path::new(&dir).join(format!("{name}.png"));
            image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                .expect("a screenshot");
            solar.hud_at = f64::NEG_INFINITY;
            solar.update_hud(t);
            notes.push_str(&format!("{name} (t={t:.1}s) {}\n", solar.hud.title));
            for (l, v) in &solar.hud.lines {
                notes.push_str(&format!("      {l:>13}  {v}\n"));
            }
            if let Some(m) = &solar.hud.manoeuvre {
                notes.push_str(&format!("      [{}] {}\n", m.name, m.text));
                for (l, v) in &m.stages {
                    notes.push_str(&format!("        {l:>18}  {v}\n"));
                }
            }
            if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                thumbs.push(image::imageops::thumbnail(&image, cell, cell));
            }
        }
        rows.push(thumbs);
    }
    let across = rows.iter().map(|r| r.len()).max().unwrap_or(0) as u32;
    let mut sheet = image::RgbaImage::new(cell * across.max(1), cell * rows.len().max(1) as u32);
    for (r, thumbs) in rows.iter().enumerate() {
        for (c, thumb) in thumbs.iter().enumerate() {
            image::imageops::overlay(
                &mut sheet,
                thumb,
                (cell * c as u32) as i64,
                (cell * r as u32) as i64,
            );
        }
    }
    sheet
        .save(std::path::Path::new(&dir).join("starbase-sheet.png"))
        .expect("a contact sheet");
    std::fs::write(std::path::Path::new(&dir).join("starbase.txt"), notes).expect("notes");
}

/// Every encounter between the legs of a jump, gates and wormholes too:
/// dropping out, five moments of it and the drive re-engaging (or the
/// ship thrown on into the next leg), to `SOLAR_SHOTS_DIR` (default
/// `target/solar-shots`): `encounter-<event>-<n>.png` and
/// `encounters-sheet.png`, a row an event. `SOLAR_ENCOUNTERS=<slugs>`
/// picks some.
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn encounters() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let only = std::env::var("SOLAR_ENCOUNTERS").ok();
    let events: Vec<voyage::Event> = voyage::Event::ALL
        .into_iter()
        .filter(|e| {
            only.as_ref()
                .is_none_or(|o| o.split(',').any(|s| s == e.slug()))
        })
        .collect();
    let variant: u64 = std::env::var("SOLAR_VARIANT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    let moments = [0.6, 0.15, 0.4, 0.6, 0.8, 0.97, 1.03];
    let cell = 216;
    let mut sheet = image::RgbaImage::new(cell * moments.len() as u32, cell * events.len() as u32);
    let mut solar = Solar::with_seed(&gpu, 11);
    wait_for_models(&mut solar, &gpu);
    for (row, event) in events.iter().enumerate() {
        solar.tour = tour_through(
            11,
            vec![voyage::Stopover {
                event: *event,
                variant,
            }],
        );
        let stop = solar
            .tour
            .segments
            .iter()
            .find(|s| s.phase == Phase::Stopover(0))
            .copied()
            .expect("a stopover");
        settle(&mut solar, &gpu, stop.start);
        for (m, &at) in moments.iter().enumerate() {
            let t = if m == 0 {
                stop.start + at
            } else if at > 1.0 {
                // Re-engaging, or thrown on into the next leg.
                stop.start
                    + stop.duration
                    + if event.portal() {
                        0.4
                    } else {
                        event.rejump() * 0.6
                    }
            } else {
                stop.start + stop.duration * at
            };
            solar.draw(&gpu, t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            shoot(
                &dir,
                &format!("encounter-{}-{m}", event.slug()),
                pixels,
                &mut sheet,
                cell,
                m as u32,
                row as u32,
            );
        }
    }
    sheet
        .save(std::path::Path::new(&dir).join("encounters-sheet.png"))
        .expect("a sheet");
}

/// Hive cubes of eight draws at the same moment of their scripts, to
/// show each is its own: `hive-cubes-sheet.png`.
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn hive_cubes() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let cell = 270;
    let mut sheet = image::RgbaImage::new(cell * 4, cell * 2);
    let mut solar = Solar::with_seed(&gpu, 11);
    wait_for_models(&mut solar, &gpu);
    for v in 0..8u64 {
        let stop = voyage::Stopover {
            event: voyage::Event::HiveCube,
            variant: v * 7919 + 3,
        };
        solar.tour = tour_through(11, vec![stop]);
        let seg = solar
            .tour
            .segments
            .iter()
            .find(|s| s.phase == Phase::Stopover(0))
            .copied()
            .expect("a stopover");
        let t = seg.start + seg.duration * 0.5;
        settle(&mut solar, &gpu, t);
        solar.draw(&gpu, t, 0);
        let pixels = read_texture(&gpu, &solar.frames[0].texture);
        shoot(
            &dir,
            &format!("hive-cube-{v}"),
            pixels,
            &mut sheet,
            cell,
            (v % 4) as u32,
            (v / 4) as u32,
        );
    }
    sheet
        .save(std::path::Path::new(&dir).join("hive-cubes-sheet.png"))
        .expect("a sheet");
}

/// Asteroid fields of the three kinds of rock (carbon, ice, iron), each
/// as a big rock nears and as it passes close, and the field at large:
/// fixed moments, so shots before and after a change line up
/// (`asteroids-sheet.png`, `asteroids-<kind>-<n>.png`).
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn asteroid_fields() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let cell = 360;
    let mut sheet = image::RgbaImage::new(cell * 3, cell * 3);
    let mut solar = Solar::with_seed(&gpu, 11);
    wait_for_models(&mut solar, &gpu);
    let mut found = 0;
    let mut variant = 0u64;
    while found < 3 {
        variant += 1;
        let stop = voyage::Stopover {
            event: voyage::Event::Asteroids,
            variant,
        };
        if voyage::shot(&stop, 0.0).extra[0] as usize != found {
            continue;
        }
        solar.tour = tour_through(11, vec![stop]);
        let seg = solar
            .tour
            .segments
            .iter()
            .find(|s| s.phase == Phase::Stopover(0))
            .copied()
            .expect("a stopover");
        settle(&mut solar, &gpu, seg.start);
        // The first big rock nearing and passing, and the field.
        let pass = voyage::shot(&stop, 0.0).objects[0][2] / 55.0;
        for (m, t) in [pass - 2.6, pass - 0.9, seg.duration * 0.45]
            .into_iter()
            .enumerate()
        {
            solar.draw(&gpu, seg.start + t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            let kind = ["carbon", "ice", "iron"][found];
            shoot(
                &dir,
                &format!("asteroids-{kind}-{m}"),
                pixels,
                &mut sheet,
                cell,
                m as u32,
                found as u32,
            );
        }
        found += 1;
    }
    sheet
        .save(std::path::Path::new(&dir).join("asteroids-sheet.png"))
        .expect("a sheet");
}

/// A planet of each of the main kinds with ground, the first such stop
/// found in the tours seeded 0 up, each high in orbit and low over its
/// horizon: fixed moments, so shots before and after a change line up
/// (`planets-sheet.png`, `planet-<kind>-<n>.png`). `SOLAR_PLANET_KINDS`
/// picks some (by name, as `Kind::name` gives them, lower case).
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn planet_gallery() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let only = std::env::var("SOLAR_PLANET_KINDS").ok();
    let kinds: Vec<Kind> = [
        Kind::Terran,
        Kind::Ocean,
        Kind::Jungle,
        Kind::Desert,
        Kind::Lava,
        Kind::Ice,
        Kind::Barren,
    ]
    .into_iter()
    .filter(|k| {
        only.as_ref()
            .is_none_or(|o| o.split(',').any(|s| s == k.name().to_lowercase()))
    })
    .collect();
    let cell = 360;
    let mut sheet = image::RgbaImage::new(cell * 3, cell * kinds.len() as u32);
    for (row, kind) in kinds.iter().enumerate() {
        let found = (0..400u64).find_map(|seed| {
            let tour = Tour::new(System::generate(seed), System::generate(seed + 1));
            let stop = tour
                .itinerary
                .iter()
                .copied()
                .find(|s| s.moon.is_none() && tour.system.planets[s.planet].kind == *kind)?;
            Some((seed, stop))
        });
        let Some((seed, stop)) = found else { continue };
        let mut solar = Solar::with_seed(&gpu, seed);
        solar.tour = Tour::new(System::generate(seed), System::generate(seed + 1));
        let high = solar
            .tour
            .segments
            .iter()
            .find(|s| s.phase == Phase::High(stop))
            .copied()
            .expect("an orbit");
        let t = high.start + high.duration * 0.5;
        settle(&mut solar, &gpu, t);
        let centre = solar.tour.system.centre(stop, t);
        let radius = solar.tour.world(stop).radius;
        let sun = normalize(scale(centre, -1.0));
        let up = [0.0, 1.0, 0.0];
        let side = normalize(cross(sun, up));
        // From the day side, a little round toward the terminator: a lit
        // three-quarter disc; then low over the lit ground, looking along
        // the horizon.
        let from = normalize(add(scale(sun, 0.8), scale(side, 0.6)));
        let whole = tour::View {
            position: add(centre, scale(from, radius * 2.9)),
            forward: scale(from, -1.0),
            up,
            ..solar.tour.view(t)
        };
        let ground = normalize(add(
            scale(sun, 0.75),
            add(scale(side, 0.45), scale(up, 0.45)),
        ));
        let along = normalize(sub(
            scale(side, -1.0),
            scale(ground, dot(scale(side, -1.0), ground)),
        ));
        let low = tour::View {
            position: add(centre, scale(ground, radius * 1.18)),
            forward: normalize(add(along, scale(ground, -0.42))),
            up: ground,
            ..solar.tour.view(t)
        };
        let orbit = solar.tour.view(t);
        for (m, view) in [whole, low, orbit].into_iter().enumerate() {
            let mut view = view;
            view.up = normalize(cross(normalize(cross(view.forward, view.up)), view.forward));
            (view.warp, view.reveal, view.target_shown, view.spool_light) = (0.0, 1.0, 0.0, 0.0);
            solar.view_override = Some(view);
            settle(&mut solar, &gpu, t);
            solar.draw(&gpu, t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            let name = kind.name().to_lowercase().replace(' ', "-");
            shoot(
                &dir,
                &format!("planet-{name}-{m}"),
                pixels,
                &mut sheet,
                cell,
                m as u32,
                row as u32,
            );
        }
        solar.view_override = None;
    }
    sheet
        .save(std::path::Path::new(&dir).join("planets-sheet.png"))
        .expect("a sheet");
}

/// Each fleet's design alone, its capital ship and an escort, lit
/// three-quarters: `fleets-sheet.png`, a row a design.
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn fleets() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let cell = 360;
    let mut sheet = image::RgbaImage::new(cell * 2, cell * voyage::FACTIONS.len() as u32);
    let mut solar = Solar::with_seed(&gpu, 11);
    wait_for_models(&mut solar, &gpu);
    let stop = voyage::Stopover {
        event: voyage::Event::Battle,
        variant: 17,
    };
    solar.tour = tour_through(11, vec![stop]);
    let seg = solar
        .tour
        .segments
        .iter()
        .find(|s| s.phase == Phase::Stopover(0))
        .copied()
        .expect("a stopover");
    let t = seg.start + 3.0;
    settle(&mut solar, &gpu, t);
    let mut view = solar.tour.view(t);
    let basis = view.scene.expect("a stopover").basis;
    (view.forward, view.up, view.reveal) = (basis[2], basis[1], 1.0);
    solar.view_override = Some(view);
    for faction in 0..voyage::FACTIONS.len() as u32 {
        for (column, index) in [0u32, 1].into_iter().enumerate() {
            solar.showcase = Some((faction, index));
            solar.draw(&gpu, t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            let name = voyage::FACTIONS[faction as usize].replace(' ', "-");
            shoot(
                &dir,
                &format!("fleet-{name}-{column}"),
                pixels,
                &mut sheet,
                cell,
                column as u32,
                faction,
            );
        }
    }
    sheet
        .save(std::path::Path::new(&dir).join("fleets-sheet.png"))
        .expect("a sheet");
}

/// Battles of six draws (other fleets each), a row each: the fight,
/// the blast, and its shock rippling out and shoving the ships aside:
/// `battles-sheet.png`.
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn battles() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let cell = 270;
    let after = [-3.0, 0.25, 0.9, 1.8];
    let mut sheet = image::RgbaImage::new(cell * after.len() as u32, cell * 6);
    let mut solar = Solar::with_seed(&gpu, 11);
    wait_for_models(&mut solar, &gpu);
    for v in 0..6u64 {
        let stop = voyage::Stopover {
            event: voyage::Event::Battle,
            variant: v * 104_729 + 17,
        };
        solar.tour = tour_through(11, vec![stop]);
        let seg = solar
            .tour
            .segments
            .iter()
            .find(|s| s.phase == Phase::Stopover(0))
            .copied()
            .expect("a stopover");
        let kill = voyage::shot(&stop, 0.0).objects[1][2];
        settle(&mut solar, &gpu, seg.start);
        for (m, at) in after.iter().enumerate() {
            let t = seg.start + kill + at;
            solar.draw(&gpu, t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            shoot(
                &dir,
                &format!("battle-{v}-{m}"),
                pixels,
                &mut sheet,
                cell,
                m as u32,
                v as u32,
            );
        }
    }
    sheet
        .save(std::path::Path::new(&dir).join("battles-sheet.png"))
        .expect("a sheet");
}

/// What is seen inside a leg, each through two drives' light:
/// `fluxes-sheet.png`, a row a flux.
#[test]
#[ignore = "writes screenshots; needs a working GPU adapter"]
fn fluxes() {
    let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let cell = 270;
    let mut sheet = image::RgbaImage::new(cell * 4, cell * voyage::Flux::ALL.len() as u32);
    let mut solar = Solar::with_seed(&gpu, 11);
    wait_for_models(&mut solar, &gpu);
    let stop = voyage::Stopover {
        event: voyage::Event::Asteroids,
        variant: 1,
    };
    solar.tour = tour_through(11, vec![stop]);
    for (row, flux) in voyage::Flux::ALL.into_iter().enumerate() {
        solar.tour.passage.legs[0].flux = Some(flux);
        let leg = solar
            .tour
            .segments
            .iter()
            .find(|s| s.phase == Phase::Jump(0))
            .copied()
            .expect("a leg");
        settle(&mut solar, &gpu, leg.start);
        for (m, (drive, at)) in [(1, 0.4), (1, 0.6), (3, 0.5), (6, 0.5)]
            .into_iter()
            .enumerate()
        {
            solar.drive_override = Some(drive);
            if drive == 6 {
                solar.drive_override = None;
                solar.tour.passage.legs[0].look = 6;
            }
            let t = leg.start + leg.duration * at;
            solar.draw(&gpu, t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            shoot(
                &dir,
                &format!("flux-{}-{m}", flux.slug()),
                pixels,
                &mut sheet,
                cell,
                m as u32,
                row as u32,
            );
        }
        solar.tour.passage.legs[0].look = solar.tour.drive().0;
    }
    sheet
        .save(std::path::Path::new(&dir).join("fluxes-sheet.png"))
        .expect("a sheet");
}

/// One finished frame and its HUD at the first moment whose name holds
/// `moment`, for the UI previews.
pub(crate) fn render_once(seed: u64, moment: &str) -> (Vec<u8>, Hud, Vec<labels::BodyLabel>) {
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let mut solar = Solar::with_seed(&gpu, seed);
    let t = moments(&solar.tour)
        .into_iter()
        .find(|(label, _)| label.ends_with(moment))
        .map_or(60.0, |(_, t)| t);
    settle(&mut solar, &gpu, t);
    solar.draw(&gpu, t, 0);
    solar.hud_at = f64::NEG_INFINITY;
    solar.update_hud(t);
    solar.update_labels(t);
    (
        read_texture(&gpu, &solar.frames[0].texture),
        solar.hud.clone(),
        solar.labels.clone(),
    )
}

/// A frame at `moment` as `render_once` draws it, with incident
/// `incident` (`console::INCIDENTS`) raised on the HUD a few seconds
/// before, and what the ship's terminal shows then.
pub(crate) fn render_console(
    seed: u64,
    moment: &str,
    incident: usize,
) -> (Vec<u8>, Hud, console::Console) {
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let mut solar = Solar::with_seed(&gpu, seed);
    let t = moments(&solar.tour)
        .into_iter()
        .find(|(label, _)| label.ends_with(moment))
        .map_or(60.0, |(_, t)| t);
    settle(&mut solar, &gpu, t);
    solar.draw(&gpu, t, 0);
    solar.raised = Some((t + solar.visit_start - 3.0, incident));
    solar.hud_at = f64::NEG_INFINITY;
    solar.update_hud(t);
    (
        read_texture(&gpu, &solar.frames[0].texture),
        solar.hud.clone(),
        solar.console.clone(),
    )
}

/// Orders from the terminal: a world ahead sets the tour on to its
/// transfer, a star sends the ship to its jump point spooling for it, an
/// encounter drops it out into that one, and an incident with a change
/// of course swaps the next jump's first stopover without a cut.
#[test]
#[ignore = "needs a GPU adapter"]
fn the_terminal_is_obeyed() {
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let mut solar = Solar::with_seed(&gpu, 11);
    let seconds = 5.0;
    let _ = solar.catch_up(seconds);
    solar.update_hud(seconds);
    assert!(!solar.console.destinations.is_empty());
    assert_eq!(solar.console.encounters.len(), voyage::Event::ALL.len() + 1);

    let last = solar.tour.itinerary.len() - 1;
    solar.obey(console::Order::Stop(last), seconds);
    let t = seconds - solar.visit_start;
    assert_eq!(solar.tour.current(t), Some(solar.tour.itinerary[last]));

    let visit = solar.visit;
    let target = (visit + 2..).find(|&v| !relay(solar.base_seed, v)).unwrap();
    solar.obey(console::Order::Star(target), seconds + 1.0);
    let t = seconds + 1.0 - solar.visit_start;
    assert_eq!(solar.tour.segment(t).0.phase, Phase::Spool);
    assert_eq!(
        solar.tour.destination().seed,
        system_for(solar.base_seed, target).seed
    );
    let event = voyage::Event::ALL
        .iter()
        .position(|&e| e == voyage::Event::Comet)
        .unwrap();
    solar.obey(console::Order::Encounter(Some(event)), seconds + 2.0);
    assert_eq!(solar.tour.passage.stops[0].event, voyage::Event::Comet);
    let t = seconds + 2.0 - solar.visit_start;
    assert_eq!(solar.tour.segment(t).0.phase, Phase::Jump(0));
    assert_eq!(solar.tour.segment(t + 3.0).0.phase, Phase::Stopover(0));
    assert_eq!(
        solar.tour.destination().seed,
        system_for(solar.base_seed, target).seed
    );
    let ends = solar.visit_start + solar.tour.total;
    let _ = solar.catch_up(ends + 0.5);
    assert_eq!(solar.visit, target);
    assert_eq!(
        solar.tour.system.seed,
        system_for(solar.base_seed, target).seed
    );

    // A distress beacon before the jump: the derelict next.
    let beacon = console::INCIDENTS
        .iter()
        .position(|i| i.divert == Some(voyage::Event::Derelict))
        .unwrap();
    let seconds = solar.visit_start + 20.0;
    let before = solar.tour.view(20.0).position;
    solar.obey(console::Order::Incident(Some(beacon)), seconds);
    assert_eq!(solar.tour.passage.stops[0].event, voyage::Event::Derelict);
    assert_eq!(solar.tour.view(20.0).position, before);
    solar.update_hud(20.0);
    assert!(solar.hud.notice.is_some());
}

/// Fragment counts for the Pi estimate: every draw of one frame at each
/// key moment of a visit, for each seed in `PI_ESTIMATE_SEEDS` (default
/// 5,23), and every draw of each frame while the first planet streams in.
/// Written to `target/pi-estimate/solar.tsv`; `deploy/pi-estimate.sh`
/// turns them into milliseconds on the Pi.
#[test]
#[ignore = "counts fragments for the Pi estimate; needs a GPU adapter with pipeline statistics"]
fn pi_estimate() {
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    assert!(
        crate::gpu::start_counting(&gpu.device),
        "this adapter cannot count fragments ({})",
        gpu.adapter
    );
    let seeds: Vec<u64> = std::env::var("PI_ESTIMATE_SEEDS")
        .unwrap_or_else(|_| "5,23".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let mut out = String::from("scene\tmoment\tframe\tlabel\tcovered\tkept\n");
    let mut push = |moment: &str, frame: usize, counts: Vec<(String, u64, u64)>| {
        for (label, covered, kept) in counts {
            out.push_str(&format!(
                "solar\t{moment}\t{frame}\t{label}\t{covered}\t{kept}\n"
            ));
        }
    };
    for seed in seeds {
        let mut solar = Solar::with_seed(&gpu, seed);
        // Streaming the first planet in: every frame's draws.
        let mut frame = 0;
        loop {
            solar.stream(1.0);
            solar.draw(&gpu, 1.0, frame % SLOTS);
            push(
                &format!("seed{seed} streaming"),
                frame,
                crate::gpu::take_counts(&gpu),
            );
            frame += 1;
            if !solar.streamer.busy() || frame > 4000 {
                break;
            }
        }
        for (label, t) in moments(&solar.tour) {
            settle(&mut solar, &gpu, t);
            crate::gpu::take_counts(&gpu);
            solar.draw(&gpu, t, 0);
            push(
                &format!("seed{seed} {label}"),
                0,
                crate::gpu::take_counts(&gpu),
            );
        }
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/pi-estimate");
    std::fs::create_dir_all(&dir).expect("an output directory");
    std::fs::write(dir.join("solar.tsv"), out).expect("the counts");
}

/// Frame times on whatever adapter is present.
///
/// By default: how long the first planet and sky take to stream in, then the
/// frame time at each key moment of seed `SWEEP_SEED` (default 5), the
/// heaviest views. With `SWEEP_STEP` set, a sweep instead: a frame every
/// `SWEEP_STEP` seconds from `SWEEP_FROM` (default 1) to `SWEEP_TO` (default
/// the whole tour) of seed `SWEEP_SEED` (default 11; `SWEEP_DRIVE` 1..5 for
/// another jump drive), each with its phase, background and draws: where the
/// time goes through a whole visit.
#[test]
#[ignore = "a benchmark; needs a working GPU adapter"]
fn frame_time() {
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let var = |k: &str| std::env::var(k).ok().and_then(|s| s.parse::<f64>().ok());
    let Some(step) = var("SWEEP_STEP") else {
        let seed = var("SWEEP_SEED").map_or(5, |s| s as u64);
        let mut solar = Solar::with_seed(&gpu, seed);
        let started = std::time::Instant::now();
        let mut frames = 0;
        let t0 = 1.0;
        loop {
            solar.stream(t0);
            solar.draw(&gpu, t0, frames % SLOTS);
            frames += 1;
            if !solar.streamer.busy() || frames > 5000 {
                break;
            }
        }
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        println!(
            "solar: first planet and sky streamed in {frames} frames, {:.0} ms",
            started.elapsed().as_secs_f64() * 1e3
        );
        for (label, t) in moments(&solar.tour) {
            settle(&mut solar, &gpu, t);
            let n = 30;
            let started = std::time::Instant::now();
            for i in 0..n {
                solar.draw(&gpu, t + i as f64 * 0.05, i % SLOTS);
            }
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
            println!(
                "solar {label}: {:.2} ms a frame",
                started.elapsed().as_secs_f64() * 1e3 / n as f64
            );
        }
        return;
    };
    let seed = var("SWEEP_SEED").map_or(11, |s| s as u64);
    let mut solar = Solar::with_seed(&gpu, seed);
    // With the meshes, as the app has them seconds in: without, the
    // stopovers that need them are swapped for others.
    wait_for_models(&mut solar, &gpu);
    solar.tour = tour_for(seed, 0, true, None);
    solar.drive_override = var("SWEEP_DRIVE").map(|d| d as u32);
    let end = var("SWEEP_TO").unwrap_or(solar.tour.total);
    let mut t = var("SWEEP_FROM").unwrap_or(1.0);
    while t < end {
        settle(&mut solar, &gpu, t);
        let n = 10;
        let started = std::time::Instant::now();
        let mut cpu = 0.0;
        for i in 0..n {
            let drawn = std::time::Instant::now();
            solar.draw(&gpu, t + i as f64 * 0.05, i % SLOTS);
            cpu += drawn.elapsed().as_secs_f64();
        }
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        println!(
            "sweep t={t:.0} {:.2} ms (cpu {:.2}) {:?} {}",
            started.elapsed().as_secs_f64() * 1e3 / n as f64,
            cpu * 1e3 / n as f64,
            solar.tour.segment(t).0.phase,
            solar.drawn.borrow()
        );
        t += step;
    }
}

/// What each draw of a jump costs, in one process: for every encounter
/// (`SOLAR_ENCOUNTERS=<slugs>`, default all) on seed `SWEEP_SEED`
/// (default 23), a moment every `SWEEP_STEP` seconds (default 3) from
/// the drive spooling up to the next leg, timed with every draw and
/// then with each part of the frame left out. Prints a line a timing:
/// `cost <event> <phase> <t> <part> <ms>`, `all` for the whole frame.
#[test]
#[ignore = "a benchmark; needs a working GPU adapter"]
fn jump_costs() {
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    let var = |k: &str, d: f64| {
        std::env::var(k)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(d)
    };
    let seed = var("SWEEP_SEED", 23.0) as u64;
    let step = var("SWEEP_STEP", 3.0);
    let only = std::env::var("SOLAR_ENCOUNTERS").ok();
    let mut solar = Solar::with_seed(&gpu, seed);
    wait_for_models(&mut solar, &gpu);
    for event in voyage::Event::ALL {
        if only
            .as_ref()
            .is_some_and(|o| !o.split(',').any(|s| s == event.slug()))
        {
            continue;
        }
        let mut parts = vec![
            "Solar event ",
            "Solar spool",
            "Solar warp",
            "Solar drop",
            "Solar ship overlay",
            "Solar space",
            "Solar glare",
            "Solar flux",
        ];
        match event {
            voyage::Event::Battle => {
                parts.extend(["Solar ships", "Solar model", "Solar battle effects"])
            }
            voyage::Event::Asteroids | voyage::Event::Comet => {
                parts.extend(["Solar rocks", "Solar model"])
            }
            voyage::Event::Gate | voyage::Event::HiveCube | voyage::Event::Derelict => {
                parts.push("Solar model")
            }
            _ => {}
        }
        // Finer parts to time as well (parts of draws' labels, comma
        // separated): `SOLAR_PARTS="leviathans fins,battle effects beams"`.
        let extra = std::env::var("SOLAR_PARTS").unwrap_or_default();
        parts.extend(extra.split(',').filter(|p| !p.is_empty()));
        solar.tour = tour_through(seed, vec![voyage::Stopover { event, variant: 7 }]);
        let segments: Vec<_> = solar
            .tour
            .segments
            .iter()
            .filter(|s| {
                matches!(
                    s.phase,
                    Phase::Spool | Phase::Jump(_) | Phase::Stopover(_) | Phase::Rejump(_)
                )
            })
            .copied()
            .collect();
        for segment in segments {
            let mut t = segment.start + 0.05;
            while t < segment.start + segment.duration {
                settle(&mut solar, &gpu, t);
                let time = |solar: &mut Solar, skip: &[&str]| {
                    crate::gpu::skip(skip);
                    let n = 10;
                    solar.draw(&gpu, t, 0);
                    gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
                    let started = std::time::Instant::now();
                    for i in 0..n {
                        solar.draw(&gpu, t + i as f64 * 0.05, i % SLOTS);
                    }
                    gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
                    started.elapsed().as_secs_f64() * 1e3 / n as f64
                };
                let all = time(&mut solar, &[]);
                let phase = format!("{:?}", segment.phase);
                println!("cost {} {phase} {t:.1} all {all:.2}", event.slug());
                for part in &parts {
                    let without = time(&mut solar, &[part]);
                    println!(
                        "cost {} {phase} {t:.1} {} {:.2}",
                        event.slug(),
                        part.trim().replace(' ', "_"),
                        all - without
                    );
                }
                crate::gpu::skip(&[]);
                t += step;
            }
        }
    }
}
