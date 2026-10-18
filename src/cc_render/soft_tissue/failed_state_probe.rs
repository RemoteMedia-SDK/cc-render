//! Offline precision probe for states captured by `failed_state::record`.
//! Child of `implicit` so it can assemble the same private sparse system.
use super::*;
use serde_json::{json, Value};

fn f(v: &Value) -> f64 {
    f64::from_bits(v.as_u64().unwrap())
}
fn vec3(v: &Value) -> DVec3 {
    DVec3::new(f(&v[0]), f(&v[1]), f(&v[2]))
}
fn vecs(v: &Value) -> Vec<DVec3> {
    v.as_array().unwrap().iter().map(vec3).collect()
}
fn frame(v: &Value) -> Frame {
    let r = &v["rotation"];
    Frame {
        translation: vec3(&v["translation"]),
        rotation: DQuat::from_xyzw(f(&r[0]), f(&r[1]), f(&r[2]), f(&r[3])),
    }
}
fn norm(v: &[DVec3]) -> f64 {
    v.iter().map(|x| x.length_squared()).sum::<f64>().sqrt()
}
fn ulp(x: f64) -> f64 {
    f64::from_bits(x.abs().to_bits() + 1) - x.abs()
}

struct Loaded {
    tissue: Tissue,
    interval: f64,
    end_frame: Frame,
    gravity: DVec3,
    contacts: Vec<PlaneContact>,
    iterate: Vec<DVec3>,
    recorded_error: String,
}

fn load(state: &Value, asset: &crate::cc_render::soft_tissue::TissueAsset) -> Loaded {
    let rest_0 = vec3(&state["rest_0"]);
    let region = asset
        .regions
        .iter()
        .find(|r| r.rest_positions.len() == state["node_count"].as_u64().unwrap() as usize
            && DVec3::from(r.rest_positions[0]) == rest_0)
        .expect("dump matches one asset region");
    let mut tissue = region.solver(frame(&state["frame"])).unwrap();
    tissue.positions = vecs(&state["positions"]);
    tissue.velocities = vecs(&state["velocities"]);
    tissue.frame = frame(&state["frame"]);
    tissue.previous = state["previous"].as_object().map(|old| History {
        positions: vecs(&old["positions"]),
        velocities: vecs(&old["velocities"]),
        dt: f(&old["dt"]),
    });
    let contacts = state["contacts"].as_array().unwrap().iter().map(|c| PlaneContact {
        point: crate::cc_render::soft_tissue::EmbeddedPoint {
            tetrahedron: c["tetrahedron"].as_u64().unwrap() as usize,
            barycentric: std::array::from_fn(|i| f(&c["barycentric"][i])),
            influence: f(&c["influence"]),
            offset: vec3(&c["offset"]),
        },
        surface_point: vec3(&c["surface_point"]),
        normal: vec3(&c["normal"]),
        stiffness_n_per_m: f(&c["stiffness_n_per_m"]),
    }).collect();
    Loaded {
        tissue,
        interval: f(&state["interval_s"]),
        end_frame: frame(&state["end_frame"]),
        gravity: vec3(&state["gravity"]),
        contacts,
        iterate: vecs(&state["iterate"]),
        recorded_error: state["error"].as_str().unwrap().to_owned(),
    }
}

/// `CC_TISSUE_FAILED_STATE=<dump> CC_TISSUE_FAILED_STATE_ASSET=<asset>
/// cargo test --offline --lib failed_state_precision -- --ignored --nocapture`
#[test]
#[ignore = "requires a recorded failed state"]
fn failed_state_precision() {
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("CC_TISSUE_FAILED_STATE").unwrap()).unwrap(),
    ).unwrap();
    let asset: crate::cc_render::soft_tissue::TissueAsset = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("CC_TISSUE_FAILED_STATE_ASSET").unwrap()).unwrap(),
    ).unwrap();
    let loaded = load(&state, &asset);
    let mut report = serde_json::Map::new();

    // 1. The reloaded step must fail identically, or nothing below applies.
    let mut replay = loaded.tissue.clone();
    let error = replay
        .step_with_contacts(loaded.interval, loaded.end_frame, loaded.gravity, &loaded.contacts)
        .unwrap_err()
        .to_string();
    assert_eq!(error, loaded.recorded_error, "replay differs from the recording");
    let tissue = &loaded.tissue;
    let step = IntegrationStep::new(tissue, loaded.interval, loaded.gravity);
    let (dt, x, frame) = (step.dt, &loaded.iterate, loaded.end_frame);
    let (force_limit, correction_limit) =
        nonlinear_limits(dt, tissue.material.time_integration);
    report.insert("replay_error".into(), json!(error));
    report.insert("interval_s".into(), json!(loaded.interval));
    report.insert("previous_dt_s".into(), json!(tissue.previous.as_ref().map(|p| p.dt)));
    report.insert("effective_bdf2_dt_s".into(), json!(dt));
    report.insert("force_limit_n".into(), json!(force_limit));
    report.insert("correction_limit_m".into(), json!(correction_limit));

    // 2. Gradient decomposition at the final iterate (free nodes only).
    let free: Vec<_> = (0..x.len()).filter(|i| tissue.inverse_mass[*i] > 0.0).collect();
    let lin = |contacts: &[PlaneContact], damping: &Dissipation| {
        tissue.linearize(x, &step.predicted, frame, dt, contacts, damping)
    };
    let full = lin(&loaded.contacts, &step.damping);
    let no_contact = lin(&[], &step.damping);
    let only_elements = Dissipation {
        elements: step.damping.elements.iter().map(|e| super::viscosity::Element {
            tet: e.tet, history_strain: e.history_strain, history_log_volume: e.history_log_volume,
            shear: e.shear, bulk: e.bulk,
        }).collect(),
        ..Default::default()
    };
    let no_damping = lin(&[], &Dissipation::default());
    let with_elements = lin(&[], &only_elements);
    let inertia: Vec<_> = (0..x.len()).map(|i| if tissue.inverse_mass[i] > 0.0 {
        tissue.masses[i] / (dt * dt) * (x[i] - step.predicted[i])
    } else { DVec3::ZERO }).collect();
    let diff = |a: &[DVec3], b: &[DVec3]| a.iter().zip(b).map(|(a, b)| *a - *b).collect::<Vec<_>>();
    let terms = [
        ("inertia", inertia.clone()),
        ("elastic_pressure_attachment", diff(&no_damping.gradient, &inertia)),
        ("viscous_elements", diff(&with_elements.gradient, &no_damping.gradient)),
        ("viscous_pressure", diff(&no_contact.gradient, &with_elements.gradient)),
        ("contact", diff(&full.gradient, &no_contact.gradient)),
    ];
    report.insert("residual_n".into(), json!(norm(&full.gradient)));
    report.insert("term_norms_n".into(), json!(terms.iter()
        .map(|(name, g)| (name.to_string(), json!(norm(g)))).collect::<serde_json::Map<_, _>>()));
    // Sum of absolute term magnitudes bounds cancellation in the total.
    let magnitude: Vec<f64> = (0..x.len()).map(|i| terms.iter().map(|(_, g)| g[i].length()).sum()).collect();
    let mut worst: Vec<_> = free.iter().copied().collect();
    worst.sort_by(|a, b| full.gradient[*b].length().total_cmp(&full.gradient[*a].length()));
    report.insert("largest_residual_nodes".into(), json!(worst.iter().take(8).map(|&i| json!({
        "node": i,
        "residual_n": full.gradient[i].length(),
        "term_magnitude_sum_n": magnitude[i],
        "terms_n": terms.iter().map(|(n, g)| (n.to_string(), json!(g[i].length()))).collect::<serde_json::Map<_,_>>(),
        "diagonal_norm_n_per_m": full.diagonal[i].to_cols_array().iter().map(|v| v*v).sum::<f64>().sqrt(),
        "position": x[i].to_array(),
    })).collect::<Vec<_>>()));
    let residual_share: f64 = worst.iter().take(8).map(|&i| full.gradient[i].length_squared()).sum::<f64>()
        / full.gradient.iter().map(|g| g.length_squared()).sum::<f64>();
    report.insert("largest_8_nodes_residual_share".into(), json!(residual_share));

    // 3. Representability floor: residual left by rounding an exact
    // solution to the f64 grid, H * r with r uniform in +-ulp/2.
    let mut seed = 0x9e3779b97f4a7c15_u64;
    let mut random = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5 };
    let floors: Vec<f64> = (0..64).map(|_| {
        let r: Vec<_> = (0..x.len()).map(|i| if tissue.inverse_mass[i] > 0.0 {
            DVec3::new(random() * ulp(x[i].x), random() * ulp(x[i].y), random() * ulp(x[i].z))
        } else { DVec3::ZERO }).collect();
        norm(&full.multiply(&r))
    }).collect();
    let rms = (floors.iter().map(|v| v * v).sum::<f64>() / floors.len() as f64).sqrt();
    report.insert("representability_floor_rms_n".into(), json!(rms));
    report.insert("representability_floor_max_n".into(), json!(floors.iter().copied().fold(0.0, f64::max)));

    // 4. Evaluation noise: identical x, same physics, gradient assembled
    // after a translation that is exact in y (every |y| in [1,2), shift is a
    // multiple of 2^-4). Other axes are unchanged. Differences are rounding.
    let in_unit_binade = x.iter().chain(&step.predicted).all(|p| (1.0..2.0).contains(&p.y));
    report.insert("y_in_unit_binade".into(), json!(in_unit_binade));
    if in_unit_binade {
        let mean_y = x.iter().map(|p| p.y).sum::<f64>() / x.len() as f64;
        let shift = DVec3::new(0.0, (mean_y * 16.0).round() / 16.0, 0.0);
        let mut moved = tissue.clone();
        moved.frame.translation -= shift;
        let xs: Vec<_> = x.iter().map(|p| *p - shift).collect();
        let ps: Vec<_> = step.predicted.iter().map(|p| *p - shift).collect();
        assert!(xs.iter().zip(x).all(|(a, b)| *a + shift == *b));
        let mut end = frame;
        end.translation -= shift;
        let contacts: Vec<_> = loaded.contacts.iter().map(|c| {
            let mut c = *c;
            c.surface_point -= shift;
            c.point.offset -= (1.0 - c.point.influence) * shift;
            c
        }).collect();
        let centered = moved.linearize(&xs, &ps, end, dt, &contacts, &step.damping);
        let centered_no_contact = moved.linearize(&xs, &ps, end, dt, &[], &step.damping);
        report.insert("centered_residual_n".into(), json!(norm(&centered.gradient)));
        report.insert("world_vs_centered_gradient_difference_n".into(),
            json!(norm(&diff(&full.gradient, &centered.gradient))));
        report.insert("world_vs_centered_noncontact_difference_n".into(),
            json!(norm(&diff(&no_contact.gradient, &centered_no_contact.gradient))));
    }

    // 5. Continue Newton from the failed iterate with the unchanged
    // objective and record whether the residual decreases or stagnates.
    let objective = |p: &[DVec3]| {
        tissue.objective(p, &step.predicted, frame, dt)
            + loaded.contacts.iter().map(|c| c.energy(p, &tissue.tets)).sum::<f64>()
            + step.damping.energy(p, &tissue.tets, &tissue.pressure_patches)
    };
    let mut current = x.clone();
    let mut energy = objective(&current);
    let mut history = Vec::new();
    for _ in 0..12 {
        let system = tissue.linearize(&current, &step.predicted, frame, dt, &loaded.contacts, &step.damping);
        let delta = system.direction().unwrap();
        let predicted_change = system.multiply(&delta);
        let linear: Vec<_> = system.gradient.iter().zip(&predicted_change).map(|(g, a)| *g + *a).collect();
        let candidate: Vec<_> = current.iter().zip(&delta).map(|(p, d)| *p + *d).collect();
        let moved_bits = candidate.iter().zip(&current).filter(|(a, b)| a != b).count();
        let next = tissue.linearize(&candidate, &step.predicted, frame, dt, &loaded.contacts, &step.damping);
        let candidate_energy = objective(&candidate);
        history.push(json!({
            "residual_n": norm(&system.gradient),
            "correction_max_m": delta.iter().map(|d| d.length()).fold(0.0, f64::max),
            "linear_model_residual_n": norm(&linear),
            "full_step_residual_n": norm(&next.gradient),
            "full_step_energy_change_j": candidate_energy - energy,
            "expected_energy_change_j": 0.5 * dot(&system.gradient, &delta),
            "nodes_changed": moved_bits,
        }));
        current = candidate;
        energy = candidate_energy;
    }
    report.insert("newton_continuation".into(), json!(history));

    // 5b. Per-term energy change of the first full Newton step versus the
    // first-order change predicted by that term's own gradient.
    {
        let system = tissue.linearize(x, &step.predicted, frame, dt, &loaded.contacts, &step.damping);
        let delta = system.direction().unwrap();
        let candidate: Vec<_> = x.iter().zip(&delta).map(|(p, d)| *p + *d).collect();
        let actual: Vec<_> = candidate.iter().zip(x).map(|(a, b)| *a - *b).collect();
        let inertial = |p: &[DVec3]| p.iter().zip(&step.predicted).zip(&tissue.masses)
            .map(|((x, q), m)| 0.5 * m * x.distance_squared(*q) / (dt * dt)).sum::<f64>();
        let energies: [(&str, Box<dyn Fn(&[DVec3]) -> f64>); 5] = [
            ("inertia", Box::new(inertial)),
            ("elastic_pressure_attachment", Box::new(|p: &[DVec3]| tissue.objective(p, &step.predicted, frame, dt) - inertial(p))),
            ("viscous_elements", Box::new(|p: &[DVec3]| only_elements.energy(p, &tissue.tets, &[]))),
            ("viscous_pressure", Box::new(|p: &[DVec3]| step.damping.energy(p, &tissue.tets, &tissue.pressure_patches)
                - only_elements.energy(p, &tissue.tets, &[]))),
            ("contact", Box::new(|p: &[DVec3]| loaded.contacts.iter().map(|c| c.energy(p, &tissue.tets)).sum::<f64>())),
        ];
        report.insert("first_step_energy_terms".into(), json!(energies.iter().zip(&terms).map(|((name, e), (_, g))| {
            (name.to_string(), json!({
                "value_j": e(x),
                "actual_change_j": e(&candidate) - e(x),
                "gradient_predicted_change_j": dot(g, &actual),
            }))
        }).collect::<serde_json::Map<_, _>>()));
    }

    // 5c. The production iteration from the original warm start, recording
    // which line-search scale `descend` accepts (same objective and limits).
    {
        let mut current: Vec<_> = tissue.positions.iter().zip(&tissue.velocities)
            .map(|(p, v)| *p + loaded.interval * *v).collect();
        tissue.place_fixed_attachments(&mut current, frame);
        let mut energy = objective(&current);
        let mut trace = Vec::new();
        let mut factor = None;
        for iteration in 0..=tissue.material.iterations {
            if iteration == 2 { factor = None; }
            let system = tissue.linearize(&current, &step.predicted, frame, dt, &loaded.contacts, &step.damping);
            let delta = system.direction_cached(&mut factor).unwrap();
            let slope = dot(&system.gradient, &delta);
            let derivative = |p: &[DVec3]| dot(
                &tissue.linearize(p, &step.predicted, frame, dt, &loaded.contacts, &step.damping).gradient, &delta);
            let largest = (0..delta.len()).max_by(|a, b| delta[*a].length().total_cmp(&delta[*b].length())).unwrap();
            let accepted = tissue.descend((&current, energy), &delta, slope, &objective, derivative).ok()
                .map(|(candidate, e)| {
                    let scale = (candidate[largest] - current[largest]).length() / delta[largest].length();
                    (scale, e - energy, candidate)
                });
            trace.push(json!({
                "iteration": iteration,
                "residual_n": norm(&system.gradient),
                "correction_max_m": delta.iter().map(|d| d.length()).fold(0.0, f64::max),
                "accepted_scale": accepted.as_ref().map(|a| a.0),
                "energy_change_j": accepted.as_ref().map(|a| a.1),
                "armijo_slope_j": slope,
            }));
            match accepted {
                Some((_, e, candidate)) if iteration < tissue.material.iterations => { energy += e; current = candidate; }
                _ => break,
            }
        }
        report.insert("production_iteration_trace".into(), json!(trace));
    }
    report.insert("energy_j".into(), json!(objective(x)));
    report.insert("energy_ulp_j".into(), json!(ulp(objective(x))));

    // 6. Conditioning of the final tangent (power / inverse iteration).
    let mut v: Vec<_> = free.iter().fold(vec![DVec3::ZERO; x.len()], |mut v, &i| { v[i] = DVec3::new(1.0, 0.7, -0.4); v });
    let mut largest = 0.0;
    for _ in 0..200 {
        let n = norm(&v);
        v.iter_mut().for_each(|p| *p /= n);
        let w = full.multiply(&v);
        largest = dot(&v, &w);
        v = w;
    }
    let mut inverse = System {
        pattern: full.pattern,
        couplings: full.couplings.iter().map(|c| Coupling { nodes: c.nodes, block: c.block }).collect(),
        diagonal: full.diagonal.clone(),
        inverse_diagonal: full.inverse_diagonal.clone(),
        gradient: Vec::new(),
    };
    let mut u: Vec<_> = free.iter().fold(vec![DVec3::ZERO; x.len()], |mut v, &i| { v[i] = DVec3::new(0.3, -1.0, 0.5); v });
    let mut smallest = 0.0;
    for _ in 0..60 {
        let n = norm(&u);
        u.iter_mut().for_each(|p| *p /= n);
        inverse.gradient = u.iter().map(|p| -*p).collect();
        let w = inverse.direction().unwrap();
        smallest = 1.0 / dot(&u, &w);
        u = w;
    }
    report.insert("tangent_largest_eigenvalue_n_per_m".into(), json!(largest));
    report.insert("tangent_smallest_eigenvalue_n_per_m".into(), json!(smallest));
    report.insert("tangent_condition".into(), json!(largest / smallest));
    report.insert("contacts".into(), json!(loaded.contacts.iter().map(|c| json!({
        "stiffness_n_per_m": c.stiffness_n_per_m,
        "depth_m": c.depth(x, &tissue.tets),
        "influence": c.point.influence,
        "tetrahedron": c.point.tetrahedron,
    })).collect::<Vec<_>>()));
    println!("{}", serde_json::to_string_pretty(&Value::Object(report)).unwrap());
}

#[test]
fn recorded_avatar_contact_step_converges_once_descent_uses_the_gradient() {
    // Seed-739 right-breast contact at 7666.67 ms, subdivision depth 6. The
    // energy-only line search exhausted 32 iterations at 1.1e-6 N against a
    // 1.33e-7 N limit; limits, contacts, material and history are unchanged.
    let state: Value = serde_json::from_str(include_str!("fixtures/contact_line_search_failed_state.json")).unwrap();
    let asset: crate::cc_render::soft_tissue::TissueAsset = serde_json::from_str(&std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tissue/beautiful_assistant.base-quality-stabilized-volume-prototype.json"),
    ).unwrap()).unwrap();
    let loaded = load(&state, &asset);
    assert!(loaded.recorded_error.contains("nonlinear solve did not converge"));
    let mut tissue = loaded.tissue.clone();
    let metrics = tissue
        .step_with_contacts(loaded.interval, loaded.end_frame, loaded.gravity, &loaded.contacts)
        .unwrap();
    assert_eq!(metrics.active_contacts, 2);
    let step = IntegrationStep::new(&loaded.tissue, loaded.interval, loaded.gravity);
    let (force_limit, _) = nonlinear_limits(step.dt, tissue.material.time_integration);
    let system = loaded.tissue.linearize(tissue.positions(), &step.predicted, loaded.end_frame, step.dt,
        &loaded.contacts, &step.damping);
    assert!(norm(&system.gradient) <= force_limit);
    // The committed state differs from the failed iterate only by the
    // remaining sub-picometre Newton correction.
    let moved = tissue.positions().iter().zip(&loaded.iterate).map(|(a, b)| a.distance(*b)).fold(0.0, f64::max);
    assert!(moved < 1e-12, "{moved}");
}
