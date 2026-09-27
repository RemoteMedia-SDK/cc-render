"""Supplemental source gates for forbidden production hooks.

These checks complement Rust behavior tests; they are not a complete Rust AST
capability sandbox. Changes to this gate require owner review.
"""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1] / "src/cc_render"


class MotionOwnershipTests(unittest.TestCase):
    def test_office_install_has_no_body_repair_or_push_actuator(self):
        source = (ROOT / "office/systems.rs").read_text()
        install = source.split("pub fn install(", 1)[1].split("fn spawn_office(", 1)[0]
        for forbidden in ("contact_corrections", "super::contact::", "apply_furniture_push"):
            self.assertNotIn(forbidden, install)

    def test_root_acceptance_does_not_rewrite_the_candidate(self):
        source = (ROOT / "office/controller.rs").read_text()
        resolve = source.split("pub fn resolve_root(", 1)[1].split("pub fn permits_root(", 1)[0]
        for forbidden in ("supported_root", "clamp", "let mut", "for "):
            self.assertNotIn(forbidden, resolve)
        self.assertIn("then_some(proposed)", resolve)

    def test_playback_cannot_activate_pelvis_clamp(self):
        source = (ROOT / "bevy_app/pose.rs").read_text()
        playback = source.split("pub(crate) fn apply_skeletal_pose(", 1)[1]
        self.assertNotIn("limit_pelvis_inclination(", playback)
        self.assertNotIn("CC_MAX_PELVIS", playback)


if __name__ == "__main__":
    unittest.main()
