//! Test-only stage snapshots for locating fixed-contact energy changes.
//! Capture never modifies particles; an absent environment variable is a no-op.
use super::*;
use std::cell::RefCell;
use std::io::{BufWriter, Write};

thread_local! {
    static OUTPUT: RefCell<Option<BufWriter<std::fs::File>>> = RefCell::new(
        std::env::var_os("CC_CLOTH_CONTACT_TRACE").map(|path| {
            BufWriter::new(std::fs::OpenOptions::new().write(true).create_new(true)
                .open(path).expect("create unique contact stage trace"))
        })
    );
}

pub(super) struct ContactProbe {
    before: Option<Vec<DVec3>>,
}

impl ContactProbe {
    pub(super) fn new(particles: &GripParticles<'_>) -> Self {
        Self {
            before: OUTPUT.with_borrow(|file| file.as_ref().map(|_| particles.positions.to_vec())),
        }
    }

    pub(super) fn record(&mut self, stage: &str, particles: &GripParticles<'_>) {
        let Some(before) = &mut self.before else {
            return;
        };
        if before == particles.positions {
            return;
        }
        OUTPUT.with_borrow_mut(|output| {
            let file = output.as_mut().expect("active contact probe");
            writeln!(
                file,
                "{}",
                serde_json::json!({
                    "stage":stage,"h":particles.h,
                    "before":before.iter().map(|p|p.to_array()).collect::<Vec<_>>(),
                    "after":particles.positions.iter().map(|p|p.to_array()).collect::<Vec<_>>(),
                    "previous":particles.previous.iter().map(|p|p.to_array()).collect::<Vec<_>>(),
                    "inverse_mass":particles.inverse_mass,
                })
            )
            .expect("write contact stage trace");
            file.flush().expect("flush contact stage trace");
        });
        before.copy_from_slice(particles.positions);
    }
}
