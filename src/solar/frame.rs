//! A frame of the tour: the shader's parameters and the bodies' passes for
//! a moment (`Solar::params`), and the passes that draw it (`Solar::draw`).

use super::*;

impl Solar {
    /// The shader's parameters at `t` into the visit, and the draws: which
    /// pipeline over which body, in order.
    pub(super) fn params(&self, t: f64) -> (Params, Vec<(u32, Pipe)>) {
        let view = self.view(t);
        let throttle = self.throttle(t);
        let system = &self.tour.system;
        let camera = view.position;
        let rel = |p: [f64; 3]| [p[0] - camera[0], p[1] - camera[1], p[2] - camera[2]];
        let f32x4 = |v: [f64; 3], w: f64| [v[0] as f32, v[1] as f32, v[2] as f32, w as f32];

        let star_rel = rel([0.0; 3]);
        let hidden = view.warp > 0.3 || view.deep || view.inside;
        let mut bodies: Vec<(f64, Body, Vec<Pipe>)> = Vec::new();
        if !hidden {
            let mut star = Body {
                place: f32x4(star_rel, system.star.radius),
                // A star's look: -1, then its surface's cell scale, their
                // contrast and its limb darkening.
                look: [
                    -1.0,
                    system.star.surface[0],
                    system.star.surface[1],
                    system.star.surface[2],
                ],
                // Its own frame the world's (its mesh needs one; its
                // granules lie in it).
                rot: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                ..Default::default()
            };
            if let Some(rect) = rect(&view, star_rel, system.star.radius * 1.02) {
                star.rect = rect;
                // air.w: how far round the centre the body's draws reach.
                star.air[3] = (system.star.radius * 1.02) as f32;
                bodies.push((length(star_rel), star, vec![Pipe::GlobeStar]));
            }
            for (index, planet) in system.planets.iter().enumerate() {
                let centre = planet.position(t);
                let place = rel(centre);
                let slot = self
                    .streamer
                    .ready((system.seed, Stop::planet(index).key()));
                let mut body = Body {
                    place: f32x4(place, planet.radius),
                    rot: rotation(
                        planet,
                        spin_angle(planet, t, centre),
                        t / planet.day * 0.035,
                    ),
                    look: [
                        planet.kind as u32 as f32,
                        slot.map_or(-1.0, |s| s as f32),
                        -1.0,
                        night_mode(planet.kind),
                    ],
                    air: {
                        let a = planet.kind.air();
                        [a[0], a[1], a[2], 0.0]
                    },
                    colour: flat_colour(planet),
                    ..Default::default()
                };
                let mut bound = planet.radius * if body.air[0] > 0.0 { 1.08 } else { 1.0 };
                if let Some(rings) = &planet.rings {
                    // The ring profile's row is the planet's index.
                    body.ring = [
                        rings.inner as f32,
                        rings.outer as f32,
                        rings.opacity,
                        index as f32,
                    ];
                    body.ring_colour = [rings.colour[0], rings.colour[1], rings.colour[2], 1.0];
                    bound = planet.radius * rings.outer;
                }
                if let Some(rect) = rect(&view, place, bound) {
                    body.rect = rect;
                    body.air[3] = bound as f32;
                    let eclipsed = eclipsed(system, t, centre, planet.radius, Some(index));
                    let close = length(place) < planet.radius * 1.6;
                    let passes =
                        surface_passes(planet.kind, slot, eclipsed, planet.rings.is_some(), close);
                    bodies.push((length(place), body, passes));
                }
                // Moons: a moon the size of a planet in full once its own
                // maps are in, the others once their planet's are.
                for (m, moon) in planet.moons.iter().enumerate().take(stream::MOONS as usize) {
                    let stop = Stop {
                        planet: index,
                        moon: Some(m),
                    };
                    let world = if moon.world {
                        self.streamer.ready((system.seed, stop.key()))
                    } else {
                        None
                    };
                    if world.is_none() && slot.is_none() {
                        continue;
                    }
                    let at = system.centre(stop, t);
                    let place = rel(at);
                    let angle = moon.phase + t * std::f64::consts::TAU / moon.period;
                    let air = moon.kind.air();
                    let bound = moon.radius * if air[0] > 0.0 { 1.08 } else { 1.0 };
                    let Some(rect) = rect(&view, place, bound) else {
                        continue;
                    };
                    let eclipsed = eclipsed(system, t, at, moon.radius, None);
                    let (look, passes) = match (world, slot) {
                        (Some(w), _) => (
                            [
                                moon.kind as u32 as f32,
                                w as f32,
                                -1.0,
                                night_mode(moon.kind),
                            ],
                            {
                                // The world a starbase stands on is never
                                // drawn as the ground close up: the ship
                                // comes down on the base, which hides it,
                                // and the close ground's three passes
                                // filling the frame were 30 ms on the Pi.
                                let near = length(place) < moon.radius * 1.6;
                                let mut passes = surface_passes(
                                    moon.kind,
                                    Some(w),
                                    eclipsed,
                                    false,
                                    near && !moon.starbase,
                                );
                                // Nor, as it fills the frame on the way
                                // down, with its night lights' own pass:
                                // 2 ms of 12.7 there on the Pi.
                                if moon.starbase && length(place) < moon.radius * 1.25 {
                                    passes.retain(|p| !matches!(p, Pipe::GlobeGlow(..)));
                                }
                                passes
                            },
                        ),
                        (None, Some(s)) => {
                            let air = air[0] > 0.0;
                            let mut passes = vec![Pipe::GlobeMoon { slot: s, air }];
                            if eclipsed {
                                passes.push(Pipe::Eclipse);
                            }
                            if air {
                                passes.push(Pipe::Limb);
                            }
                            (
                                [
                                    moon.kind as u32 as f32,
                                    s as f32,
                                    m as f32,
                                    night_mode(moon.kind),
                                ],
                                passes,
                            )
                        }
                        (None, None) => continue,
                    };
                    bodies.push((
                        length(place),
                        Body {
                            place: f32x4(place, moon.radius),
                            // Locked to its planet, one face toward it; the
                            // clouds of a world drift on their own.
                            rot: match self.tour.base_attitude() {
                                // A starbase holds still, its mouth on the
                                // corridor flown in by.
                                Some((base, rows)) if base == stop => {
                                    rows.map(|r| [r[0] as f32, r[1] as f32, r[2] as f32, 0.0])
                                }
                                _ => rotation(
                                    planet,
                                    angle + std::f64::consts::PI,
                                    t / moon.period * 0.2,
                                ),
                            },
                            look,
                            air: [air[0], air[1], air[2], bound as f32],
                            colour: [
                                moon.palette[0][0],
                                moon.palette[0][1],
                                moon.palette[0][2],
                                0.0,
                            ],
                            rect,
                            ..Default::default()
                        },
                        passes,
                    ));
                }
            }
        }
        // Far to near; the farthest dropped if there are too many.
        bodies.sort_by(|a, b| b.0.total_cmp(&a.0));
        if bodies.len() > MAX_BODIES {
            bodies.drain(0..bodies.len() - MAX_BODIES);
        }
        let count = bodies.len() as u32;
        let mut list = [Body::default(); MAX_BODIES];
        let mut draws = Vec::new();
        for (index, (slot, (_, body, passes))) in list.iter_mut().zip(bodies.iter()).enumerate() {
            *slot = *body;
            // The depth mask's value: nearer bodies (later, far to near) lower.
            slot.ring_colour[3] = 1.0 - (index as f32 + 1.0) / (count as f32 + 1.0);
            slot.shade = shadowers(&bodies, index, star_rel, system.star.radius);
            for pass in passes {
                draws.push((index as u32, *pass));
            }
        }

        // How much of the star's disc the bodies leave in view.
        let star_distance = length(star_rel);
        let star_dir = normalize(star_rel);
        let star_angle = (system.star.radius / star_distance).min(1.0).asin();
        let mut visible = 1.0;
        for (distance, body, _) in &bodies {
            if body.look[0] < 0.0 || *distance > star_distance {
                continue;
            }
            let dir = normalize([
                body.place[0] as f64,
                body.place[1] as f64,
                body.place[2] as f64,
            ]);
            let separation = dot(dir, star_dir).clamp(-1.0, 1.0).acos();
            let radius = (body.place[3] as f64 / distance).min(1.0).asin();
            visible *= smoothstep(radius - star_angle, radius + star_angle, separation);
        }
        if hidden {
            visible = 0.0;
        }

        let sky = sky_rotation(system.sky);
        let right = cross(view.forward, view.up);
        let params = Params {
            frame: [
                SIZE as f32 / 2.0,
                view.half_fov as f32,
                (2.0 * view.half_fov / SIZE as f64) as f32,
                count as f32,
            ],
            right: f32x4(right, 0.0),
            up: f32x4(view.up, 0.0),
            forward: f32x4(view.forward, 0.0),
            star: f32x4(star_rel, system.star.radius),
            star_colour: [
                system.star.colour[0],
                system.star.colour[1],
                system.star.colour[2],
                visible as f32,
            ],
            light: [
                system.star.light[0],
                system.star.light[1],
                system.star.light[2],
                0.0,
            ],
            sky,
            warp: [
                view.warp as f32,
                view.flare as f32,
                view.reveal as f32,
                view.clock as f32,
            ],
            burn: [throttle, t as f32, 0.0, 0.0],
            dust: [
                self.dust[0] as f32,
                self.dust[1] as f32,
                self.dust[2] as f32,
                DUST_STREAK * throttle,
            ],
            motion: f32x4(self.motion(t), 0.0),
            goal: f32x4(view.target, view.target_shown),
            goal_colour: {
                let c = self.tour.next.star.colour;
                let top = c[0].max(c[1]).max(c[2]).max(1e-3);
                [c[0] / top, c[1] / top, c[2] / top, 0.0]
            },
            spool: [
                view.spool as f32,
                view.spool_light as f32,
                self.look(view.look) as f32,
                self.look(view.drop_look) as f32,
            ],
            ev: {
                let mut ev = encounter_rows(&view);
                if let Some((faction, index)) = self.showcase {
                    ev[EV_EXTRA][1] = 1.0;
                    ev[EV_EXTRA][2] = index as f32;
                    (
                        ev[EV_OBJECT + 1][0],
                        ev[EV_OBJECT + 1][1],
                        ev[EV_OBJECT + 1][2],
                    ) = (faction as f32, faction as f32, 1e4);
                    ev[EV_OBJECT + 2][3] = -1.0;
                    (ev[EV_STATE][0], ev[EV_X][3], ev[EV_Y][3], ev[EV_Z][3]) = (0.0, 0.0, 0.0, 0.0);
                }
                fleet_rows(&mut ev);
                ev
            },
            bodies: list,
        };
        // The meshes' windows and grids, from the frame's pixel size.
        let mut params = params;
        for (body, name) in &draws {
            if name.is_globe() {
                let b = &mut params.bodies[*body as usize];
                (b.window, b.mesh) = globe_mesh(b, params.frame[2]);
                // The rings' shadow is not linear across a coarse cell.
                if b.ring[2] > 0.0 {
                    b.mesh[0] = b.mesh[0].max(RINGED_GRID as f32);
                }
            }
        }
        // The air past the limb of a body wholly ahead as an annulus.
        for (body, name) in &mut draws {
            if *name == Pipe::Limb && wholly_ahead(&params.bodies[*body as usize], &params, 1.08) {
                *name = Pipe::LimbRing;
            }
        }
        (params, draws)
    }

    pub(super) fn draw(&mut self, gpu: &Graphics, t: f64, frame: usize) {
        self.drift_dust(t);
        self.receive_models(gpu);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Solar encoder"),
            });
        self.streamer.advance(gpu, &mut encoder);
        self.bake_stars(gpu, &mut encoder);
        self.bind(gpu);
        let (params, draws) = self.params(t);
        let baked = self.starfield_slot(self.tour.system.seed).is_some() && !procedural_stars();
        let (look, drop_look) = (params.spool[2] as u32, params.spool[3] as u32);
        let event = voyage::Event::ALL
            .into_iter()
            .find(|e| e.id() == params.ev[EV_EVENT][0] as u32);
        let deep = params.ev[EV_SHIP][3] > 0.5;
        // Inside a starbase: nothing of the system, its interior drawn over
        // the clear (in the fog's colour) instead.
        let base = self.view(t).base;
        let inside = self.view(t).inside;
        // A stopover that replaces the sky, at full strength: nothing under it
        // to draw.
        let covered = event.is_some_and(|e| e.replaces_sky())
            && params.ev[EV_STATE][1] >= 1.0
            && params.warp[2] >= 1.0;
        // Looking out through its mouth, the sky beyond it.
        let sky_shows = base.is_some_and(|b| b.sky);
        let background = if inside && !(sky_shows && baked) {
            None
        } else if inside {
            Some(Pipe::SpaceBaked)
        } else if baked && params.spool[1] > 0.001 && params.warp[0] < 1.0 {
            // The drive starting up, and on into the jump's first seconds
            // until its light is all there is; while it is still aligning,
            // the sky with the star marked, all that shows then.
            match spool_stage(params.spool[0]) {
                0 => Some(Pipe::Target),
                stage => Some(Pipe::Spool {
                    drive: self.drive().0,
                    stage,
                }),
            }
        } else if params.warp[0] > 0.02 {
            let flare = params.warp[1] > 0.0;
            Some(Pipe::Warp { look, flare })
        } else if covered {
            None
        } else if baked && params.goal[3] > 0.0 {
            Some(Pipe::Target)
        } else if baked {
            Some(Pipe::SpaceBaked)
        } else {
            Some(Pipe::Space)
        };
        let spooling = matches!(background, Some(Pipe::Spool { .. }));
        // The star's glare over the baked background (`fs_space` draws its
        // own; the drives none).
        let glare = match background {
            _ if deep || inside => None,
            Some(Pipe::SpaceBaked | Pipe::Target) => glare_pass(&params),
            _ if spooling && params.warp[0] < 0.3 => glare_pass(&params),
            _ => None,
        };
        #[cfg(test)]
        {
            let background = background.map(|b| b.to_string()).unwrap_or_default();
            *self.drawn.borrow_mut() = format!(
                "{background}{} {}",
                glare.map(|g| format!(" {g}")).unwrap_or_default(),
                draws
                    .iter()
                    .map(|(b, n)| format!("{b}:{n}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            if let Some(e) = event {
                let rocks = if e.rocks() {
                    format!(" rocks{}", e.id())
                } else {
                    String::new()
                };
                self.drawn
                    .borrow_mut()
                    .push_str(&format!(" event{}{rocks}", e.id()));
            }
            if params.warp[2] < 1.0 {
                let over = format!(" dropout{drop_look}");
                self.drawn.borrow_mut().push_str(&over);
            }
        }
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&params));
        // A battle's effects, grouped by kind, and a draw for each kind.
        let mut fx_draws = Vec::new();
        let lists = if params.ev[EV_EVENT][0] as u32 == voyage::Event::Battle.id() {
            Some(battle_effects(&params.ev))
        } else {
            base.as_ref().map(starbase::effects)
        };
        if let Some(lists) = lists {
            let mut flat = Vec::new();
            for (kind, list) in lists.into_iter().enumerate() {
                let at = flat.len().min(FX_INSTANCES) as u32;
                flat.extend(list);
                flat.truncate(FX_INSTANCES);
                if flat.len() as u32 > at {
                    fx_draws.push((kind, at..flat.len() as u32));
                }
            }
            if !flat.is_empty() {
                gpu.queue
                    .write_buffer(&self.effects, 0, bytemuck::cast_slice(&flat));
            }
        }
        // A Dyson swarm's panels, placed for the frame.
        let mut panels = [0; 3];
        if event == Some(voyage::Event::DysonSwarm) {
            let placed;
            (placed, panels) = dyson_panels(&params.ev);
            gpu.queue
                .write_buffer(&self.panels, 0, bytemuck::cast_slice(&placed));
        }
        // The stopover's models, grouped by mesh: their placings into the
        // instance buffer, and a draw for each group.
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.model_draws.clear();
        if let Some(meshes) = &self.models {
            let view = self.view(t);
            let ModelScratch {
                placed,
                nearest,
                order,
                flat,
                model_draws,
            } = &mut scratch;
            model_instances(&view, placed);
            if let Some(b) = &view.base {
                starbase::instances(b, placed);
            }
            placed.truncate(MODEL_INSTANCES);
            // Inside a starbase every craft is drawn as its structures are
            // (`models::BASE`), the fleets' ships too.
            let family = |key: models::Key| match key {
                models::Key::Base { .. } => key.family(),
                _ if view.base.is_some_and(|b| b.inside) => models::BASE,
                _ => key.family(),
            };
            // A draw a mesh, the group with the nearest model first and the
            // nearest first within each: the depth test then spares the
            // pixels nearer models hide from the costly shading.
            let ship = view
                .scene
                .as_ref()
                .map(|s| s.shot.ship)
                .or(view.base.map(|b| b.ship))
                .unwrap_or([0.0; 3]);
            let near = |i: &models::Instance| {
                let d = [0, 1, 2].map(|k| i.place[k] as f64 - ship[k]);
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - i.place[3] as f64
            };
            nearest.clear();
            for (key, instance) in placed.iter() {
                let d = near(instance);
                nearest
                    .entry(*key)
                    .and_modify(|n| *n = n.min(d))
                    .or_insert(d);
            }
            order.clear();
            order.extend(
                placed
                    .iter()
                    .enumerate()
                    .map(|(i, (key, instance))| (nearest[key], *key, near(instance), i)),
            );
            order.sort_by(|a, b| {
                a.0.total_cmp(&b.0)
                    .then_with(|| a.1.cmp(&b.1))
                    .then_with(|| a.2.total_cmp(&b.2))
            });
            flat.clear();
            for &(_, key, _, i) in order.iter() {
                let instance = placed[i].1;
                let at = flat.len() as u32;
                flat.push(instance);
                let Some(range) = meshes.library.get(key) else {
                    continue;
                };
                if let Some(clusters) = meshes.library.clusters.get(&key) {
                    // A big mesh a placing at a time, only its clusters
                    // that can show, those next to each other in one draw.
                    for run in visible_runs(clusters, &instance, &params) {
                        model_draws.push((
                            family(key),
                            (run.start, run.end - run.start, range.2),
                            at..at + 1,
                        ));
                    }
                    continue;
                }
                match model_draws.last_mut() {
                    Some((f, last, instances)) if *f == family(key) && *last == range => {
                        instances.end = at + 1
                    }
                    _ => model_draws.push((family(key), range, at..at + 1)),
                }
            }
            if !flat.is_empty() {
                gpu.queue
                    .write_buffer(&meshes.instances, 0, bytemuck::cast_slice(flat));
            }
        }
        let model_draws = &scratch.model_draws;
        let Some(group) = &self.bind_group else {
            self.scratch = scratch;
            return;
        };
        // The leviathans' halos, small, before the frame's pass reads them.
        let creatures = params.ev[EV_OBJECT][3] as u32;
        let leviathans = event == Some(voyage::Event::Leviathans) && params.ev[EV_STATE][1] > 0.001;
        if leviathans {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Solar leviathan halos"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.leviathan_halos.0,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, group, &[]);
            pass.set_bind_group(1, &self.streamer.noise().group, &[]);
            if crate::gpu::counting() {
                pass.set_pipeline(&self.pipe(Pipe::ProbeLeviathan(4)).1);
                crate::gpu::probe_draw(&mut pass, 0..6, 0..creatures * 9);
            }
            let (label, pipeline) = self.pipe(Pipe::Leviathan(4));
            pass.set_pipeline(pipeline);
            crate::gpu::counted_draw(&mut pass, label, 0, 0..6, 0..creatures * 9);
        }
        // The plasma storm or the remnant, at half the resolution, likewise.
        let coarse = event.filter(|e| e.coarse() && params.ev[EV_STATE][1] > 0.001);
        if let Some(e) = coarse {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Solar coarse"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.coarse.0,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, group, &[]);
            pass.set_bind_group(1, &self.streamer.noise().group, &[]);
            if e.gas() {
                pass.set_bind_group(2, &self.gas[1].1, &[]);
            }
            let (label, pipeline) = self.pipe(Pipe::Coarse(e.id()));
            pass.set_pipeline(pipeline);
            let small = (SIZE / COARSE * (SIZE / COARSE)) as u64;
            crate::gpu::counted_draw(&mut pass, label, small, 0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Solar pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.frames[frame].view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Inside a starbase, a flat placeholder for its
                        // tunnel and bay: a dim steel blue.
                        load: wgpu::LoadOp::Clear(if inside {
                            // The interior's fog (`base_fog`), linear.
                            wgpu::Color {
                                r: 0.012,
                                g: 0.014,
                                b: 0.024,
                                a: 1.0,
                            }
                        } else {
                            wgpu::Color::BLACK
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                // Lives in the tile buffer only: never written to memory.
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, group, &[]);
            pass.set_bind_group(1, &self.streamer.noise().group, &[]);
            pass.set_index_buffer(self.grid.slice(..), wgpu::IndexFormat::Uint16);
            // First the depth mask: every body's solid disc at its rank, so
            // nothing a nearer body hides is shaded, the background included.
            let mut masked = Vec::new();
            for (body, _) in &draws {
                if !masked.contains(body) {
                    masked.push(*body);
                }
            }
            // When counting for the Pi estimate, a probe before each draw
            // counts the pixels that pass the mask: what the Pi shades.
            let counting = crate::gpu::counting();
            // The corners off the round display, before everything.
            let (label, corners) = self.pipe(Pipe::Corners);
            pass.set_pipeline(corners);
            crate::gpu::counted_draw(&mut pass, label, 0, 0..CORNER_SIDES * 6, 0..1);
            for body in &masked {
                if counting {
                    pass.set_pipeline(&self.pipe(Pipe::Probe).1);
                    crate::gpu::probe_draw(&mut pass, 0..BODY_VERTICES, *body..*body + 1);
                }
                // Wholly ahead, its silhouette's polygon; close by, traced.
                let b = &params.bodies[*body as usize];
                let (name, vertices) = if inscribable(b, &params) {
                    (Pipe::MaskInscribed, MASK_VERTICES)
                } else {
                    (Pipe::Mask, BODY_VERTICES)
                };
                let (label, mask) = self.pipe(name);
                pass.set_pipeline(mask);
                crate::gpu::counted_draw(&mut pass, label, 0, 0..vertices, *body..*body + 1);
            }
            let full = (SIZE * SIZE) as u64;
            if counting {
                pass.set_pipeline(&self.pipe(Pipe::ProbeFull).1);
                crate::gpu::probe_draw(&mut pass, 0..3, 0..1);
            }
            // Dropping out of the jump: its light ahead of the front, in
            // front of everything and writing the depth, so the background
            // and the bodies it hides are not shaded.
            let dropping = params.warp[2] < 1.0;
            if dropping {
                let (label, pipeline) = self.pipe(Pipe::DropAhead(drop_look));
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, 0, 0..DROP_SIDES * 6, 0..1);
            }
            // The drive's light rising as it engages, its start-up added
            // over it.
            if spooling && params.warp[0] > 0.0 {
                let (label, pipeline) = self.pipe(Pipe::SpoolLight(self.drive().0));
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, full, 0..3, 0..1);
            }
            if let Some(background) = background {
                let (label, pipeline) = self.pipe(background);
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, full, 0..3, 0..1);
            }
            // A stopover: its own draw over the sky (or in place of it), and
            // the rocks round the ship, in front of everything.
            if let Some(e) = event.filter(|_| params.ev[EV_STATE][1] > 0.001) {
                if leviathans {
                    // Their halos and the plankton over the whole sky, then
                    // each lobe's core, the fins and the filaments, each on
                    // its own quads.
                    pass.set_bind_group(2, &self.leviathan_halos.1, &[]);
                    let (label, pipeline) = self.pipe(Pipe::LeviathanGlow);
                    pass.set_pipeline(pipeline);
                    crate::gpu::counted_draw(&mut pass, label, full, 0..3, 0..1);
                    let fins = creatures * FIN_SLICES;
                    for (part, quads) in [(1, creatures * 9), (2, fins), (3, creatures)] {
                        if counting {
                            let probe = self.pipe(Pipe::ProbeLeviathan(part));
                            pass.set_pipeline(&probe.1);
                            crate::gpu::probe_draw(&mut pass, 0..6, 0..quads);
                        }
                        let (label, pipeline) = self.pipe(Pipe::Leviathan(part));
                        pass.set_pipeline(pipeline);
                        crate::gpu::counted_draw(&mut pass, label, 0, 0..6, 0..quads);
                    }
                } else if let Some(parts) = event_parts(e, panels) {
                    pass.set_vertex_buffer(0, self.panels.slice(..));
                    for (name, vertices, instances) in parts {
                        let (label, pipeline) = self.pipe(Pipe::EventPart(e.id(), name));
                        pass.set_pipeline(pipeline);
                        crate::gpu::counted_draw(&mut pass, label, 0, 0..vertices, instances);
                    }
                } else {
                    let (label, pipeline) = self.pipe(Pipe::Event(e.id()));
                    if coarse.is_some() {
                        pass.set_bind_group(2, &self.coarse.1, &[]);
                    } else if e.gas() {
                        pass.set_bind_group(2, &self.gas[0].1, &[]);
                    }
                    pass.set_pipeline(pipeline);
                    crate::gpu::counted_draw(&mut pass, label, full, 0..3, 0..1);
                }
                // The battle's ships, each its fleet's design in a variant of
                // its own, the one lost gone once it blows up.
                if e == voyage::Event::Battle
                    && let Some(meshes) = &self.models
                    && !crate::gpu::skipped("Solar ships")
                {
                    pass.set_pipeline(&self.pipe(Pipe::Ships).1);
                    pass.set_vertex_buffer(0, meshes.vertices.slice(..));
                    pass.set_index_buffer(meshes.indices.slice(..), wgpu::IndexFormat::Uint32);
                    let (local, kill) = (params.ev[EV_EVENT][1], params.ev[EV_OBJECT + 1][2]);
                    let lost = params.ev[EV_OBJECT + 2][3] as i32;
                    let seed = params.ev[EV_EXTRA][0] as u32;
                    for i in 0..8u32 {
                        if i as i32 == lost && local > kill {
                            continue;
                        }
                        let design = if i < 4 {
                            params.ev[EV_OBJECT + 1][0]
                        } else {
                            params.ev[EV_OBJECT + 1][1]
                        } as u32;
                        if let Some((first, count, base)) = meshes.library.for_ship(design, i, seed)
                        {
                            pass.draw_indexed(first..first + count, base, i..i + 1);
                        }
                    }
                    pass.set_index_buffer(self.grid.slice(..), wgpu::IndexFormat::Uint16);
                }
                // The other models: rocks, gates, hive cubes,
                // ring stations, debris.
                if let Some(meshes) = self
                    .models
                    .as_ref()
                    .filter(|_| !model_draws.is_empty() && !crate::gpu::skipped("Solar model"))
                {
                    pass.set_bind_group(2, &self.craters.1, &[]);
                    pass.set_vertex_buffer(0, meshes.vertices.slice(..));
                    pass.set_vertex_buffer(1, meshes.instances.slice(..));
                    pass.set_index_buffer(meshes.indices.slice(..), wgpu::IndexFormat::Uint32);
                    let mut bound = 0;
                    for (family, (first, count, base), instances) in model_draws {
                        if *family != bound {
                            pass.set_pipeline(&self.pipe(Pipe::Model(*family)).1);
                            bound = *family;
                        }
                        pass.draw_indexed(*first..*first + *count, *base, instances.clone());
                    }
                    pass.set_index_buffer(self.grid.slice(..), wgpu::IndexFormat::Uint16);
                }
                if e == voyage::Event::HiveCube {
                    // The cube's haze and tractor beam, after it: where it
                    // stands in front of them, nothing of theirs is shaded.
                    for name in ["haze", "beam"] {
                        let (label, pipeline) = self.pipe(Pipe::EventPart(e.id(), name));
                        pass.set_pipeline(pipeline);
                        crate::gpu::counted_draw(&mut pass, label, 0, 0..6, 0..1);
                    }
                }
                if e == voyage::Event::Battle && !fx_draws.is_empty() {
                    // Its fire, shields and blasts, after the ships and wreckage, tested
                    // against them at their own depths: a draw a kind.
                    // Their dim light only where no mesh is.
                    pass.set_vertex_buffer(0, self.effects.slice(..));
                    for (kind, instances) in &fx_draws {
                        for far in [false, true] {
                            let (label, pipeline) = self.pipe(Pipe::BattleFx { kind: *kind, far });
                            pass.set_pipeline(pipeline);
                            let vertices = 0..FX_KINDS[*kind].3;
                            let instances = instances.clone();
                            crate::gpu::counted_draw(&mut pass, label, 0, vertices, instances);
                        }
                    }
                }
                if e.rocks() {
                    let (label, pipeline) = self.pipe(Pipe::Rocks(e.id()));
                    pass.set_pipeline(pipeline);
                    crate::gpu::counted_draw(
                        &mut pass,
                        label,
                        (ROCKS * 300) as u64,
                        0..6,
                        0..ROCKS,
                    );
                }
            }
            // Something seen inside a leg, over the drive's light.
            let flux = params.ev[EV_STATE][2] as u32;
            if flux > 0 && params.ev[EV_STATE][3] > 0.001 && params.warp[0] > 0.5 {
                let (label, pipeline) = self.pipe(Pipe::Flux(flux));
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, full, 0..3, 0..1);
            }
            if let Some(name) = glare {
                let (vertices, probe) = match name {
                    Pipe::Glare => (GLARE_VERTICES, Pipe::ProbeGlare),
                    _ => (3, Pipe::ProbeFull),
                };
                if counting {
                    pass.set_pipeline(&self.pipe(probe).1);
                    crate::gpu::probe_draw(&mut pass, 0..vertices, 0..1);
                }
                let (label, pipeline) = self.pipe(name);
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, 0, 0..vertices, 0..1);
            }
            for (body, name) in &draws {
                // An eclipse only over where the shadows fall, if any does.
                let scissor = if *name == Pipe::Eclipse {
                    match shadow_scissor(&params, *body as usize) {
                        Some(r) => Some(r),
                        None => continue,
                    }
                } else {
                    None
                };
                if let Some([x, y, w, h]) = scissor {
                    pass.set_scissor_rect(x, y, w, h);
                }
                let instances = *body..*body + 1;
                if name.is_globe() {
                    // A mesh: its grid's triangles, probed with the same.
                    let indices = grid_range(params.bodies[*body as usize].mesh[0] as u32);
                    if counting {
                        pass.set_pipeline(&self.pipe(Pipe::ProbeGlobe).1);
                        let (probed, of) = (indices.clone(), instances.clone());
                        crate::gpu::probe_draw_indexed(&mut pass, probed, of);
                    }
                    let (label, pipeline) = self.pipe(*name);
                    pass.set_pipeline(pipeline);
                    crate::gpu::counted_draw_indexed(&mut pass, label, 0, indices, instances);
                    continue;
                }
                let (vertices, probe, probed) = match name {
                    Pipe::Ring => (RING_VERTICES, Pipe::Probe, BODY_VERTICES),
                    Pipe::Limb => (BODY_VERTICES, Pipe::ProbeStrict, BODY_VERTICES),
                    Pipe::LimbRing => (LIMB_RING_VERTICES, Pipe::ProbeLimbRing, LIMB_RING_VERTICES),
                    _ => (BODY_VERTICES, Pipe::Probe, BODY_VERTICES),
                };
                if counting {
                    pass.set_pipeline(&self.pipe(probe).1);
                    crate::gpu::probe_draw(&mut pass, 0..probed, instances.clone());
                }
                let (label, pipeline) = self.pipe(*name);
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, 0, 0..vertices, instances);
                if scissor.is_some() {
                    pass.set_scissor_rect(0, 0, SIZE, SIZE);
                }
            }
            // A starbase: its meshes (nearest first; from outside, the city
            // round its mouth, in front of the base itself), the tunnel's
            // panes with the hollow behind them, then its lights and sparks.
            if let Some(b) = base.as_ref() {
                if let Some(meshes) = self.models.as_ref().filter(|_| !model_draws.is_empty()) {
                    pass.set_bind_group(2, &self.craters.1, &[]);
                    pass.set_vertex_buffer(0, meshes.vertices.slice(..));
                    pass.set_vertex_buffer(1, meshes.instances.slice(..));
                    pass.set_index_buffer(meshes.indices.slice(..), wgpu::IndexFormat::Uint32);
                    let mut bound = 0;
                    for (family, (first, count, at), instances) in model_draws {
                        // The benchmarks leave a family out by its label.
                        if crate::gpu::skipped(&self.pipe(Pipe::Model(*family)).0) {
                            continue;
                        }
                        if *family != bound {
                            pass.set_pipeline(&self.pipe(Pipe::Model(*family)).1);
                            // A starbase's own families read its maps.
                            let maps = if *family >= models::BASE {
                                &self.city.1
                            } else {
                                &self.craters.1
                            };
                            pass.set_bind_group(2, maps, &[]);
                            bound = *family;
                        }
                        pass.draw_indexed(*first..*first + *count, *at, instances.clone());
                    }
                    pass.set_index_buffer(self.grid.slice(..), wgpu::IndexFormat::Uint16);
                }
                let (_, panes) = starbase::segments(b);
                if panes > 0 {
                    let (label, pipeline) = self.pipe(Pipe::BasePanes);
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(2, &self.city.1, &[]);
                    let quads = panes * starbase::PANES;
                    crate::gpu::counted_draw(
                        &mut pass,
                        label,
                        0,
                        0..starbase::PANE_VERTICES,
                        0..quads,
                    );
                }
                if starbase::world_shown(b) {
                    let (label, pipeline) = self.pipe(Pipe::BaseShell);
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(2, &self.city.1, &[]);
                    crate::gpu::counted_draw(&mut pass, label, 0, 0..18, 0..1);
                }
                if !fx_draws.is_empty() {
                    pass.set_vertex_buffer(0, self.effects.slice(..));
                    for (kind, instances) in &fx_draws {
                        for far in [false, true] {
                            let (label, pipeline) = self.pipe(Pipe::BattleFx { kind: *kind, far });
                            pass.set_pipeline(pipeline);
                            let vertices = 0..FX_KINDS[*kind].3;
                            let instances = instances.clone();
                            crate::gpu::counted_draw(&mut pass, label, 0, vertices, instances);
                        }
                    }
                }
            }
            // An engine burn: the dust streaking past and the glow round
            // the rim, over everything.
            if params.burn[0] > 0.0 {
                let (label, pipeline) = self.pipe(Pipe::Dust);
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, (DUST * 400) as u64, 0..6, 0..DUST);
                let (label, pipeline) = self.pipe(Pipe::Rim);
                pass.set_pipeline(pipeline);
                let ring = full - (full as f64 * 0.82 * 0.82 * std::f64::consts::FRAC_PI_4) as u64;
                crate::gpu::counted_draw(&mut pass, label, ring, 0..RIM_SIDES * 6, 0..1);
            }
            // And behind the front, over everything drawn.
            if dropping {
                let (label, pipeline) = self.pipe(Pipe::DropOut(drop_look));
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, 0, 0..DROP_SIDES * 3, 0..1);
            }
            // The ship's own effects over everything: the alarm round the
            // rim, a hit on the shield, a flash.
            let alarm = params.ev[EV_STATE][0]
                .max(params.ev[EV_X][3])
                .max(params.ev[EV_Z][3])
                .max(params.ev[EV_Y][3]);
            // A flash whitens the whole frame (drawn first: the rest is
            // dimmed as much); a hit lights round where it struck alone
            // (`vs_overlay_hit`); the alarm only the band round the rim (from
            // 0.86 of the radius out), and nothing below 0.3; the tractor
            // beam's hold only from 0.4 of it out (`BEAM_INNER`).
            let flash = params.ev[EV_Z][3];
            // Its white, linear, under half a step of the sRGB frame's darkest.
            if flash * flash > 0.5 / 255.0 / 12.92 {
                let (label, pipeline) = self.pipe(Pipe::OverlayFlash);
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, full, 0..OVERLAY_VERTICES, 0..1);
            }
            if params.ev[EV_X][3] > 0.01 {
                let (label, pipeline) = self.pipe(Pipe::OverlayHit);
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, 0, 0..HIT_SIDES * 6, 0..2);
            }
            if params.ev[EV_Y][3] > 0.01 {
                let (label, pipeline) = self.pipe(Pipe::OverlayBeam);
                pass.set_pipeline(pipeline);
                let ring = full - (full as f64 * 0.4 * 0.4 * std::f64::consts::FRAC_PI_4) as u64;
                crate::gpu::counted_draw(&mut pass, label, ring, 0..RIM_SIDES * 6, 0..1);
            } else if alarm > 0.3 {
                let (label, pipeline) = self.pipe(Pipe::OverlayRim);
                pass.set_pipeline(pipeline);
                let ring = full - (full as f64 * 0.82 * 0.82 * std::f64::consts::FRAC_PI_4) as u64;
                crate::gpu::counted_draw(&mut pass, label, ring, 0..RIM_SIDES * 6, 0..1);
            }
        }
        self.scratch = scratch;
        gpu.submit(encoder.finish());
    }

    /// The pipeline `key` and its label.
    pub(super) fn pipe(&self, key: Pipe) -> &(String, wgpu::RenderPipeline) {
        self.pipelines
            .get(&key)
            .unwrap_or_else(|| panic!("no solar pipeline `{key}`"))
    }
}
