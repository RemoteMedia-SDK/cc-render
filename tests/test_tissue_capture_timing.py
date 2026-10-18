"""Verify timing and pixel ordering through the actual diagnostic video codec."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts/avatars/retime_tissue_capture.py"
SPEC = importlib.util.spec_from_file_location("retime_tissue_capture", SCRIPT)
retime = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(retime)


def rows(timestamps):
    return [{"frame": index, "tissue_pts_ms": pts} for index, pts in enumerate(timestamps)]


class TissueCaptureTimingTests(unittest.TestCase):
    def test_incomplete_or_discontinuous_timing_is_rejected(self):
        for timestamps in ([None, None], [50, 50], [100, 99], [50, None, 100], [50, 1.5]):
            with self.subTest(timestamps=timestamps), self.assertRaises(ValueError):
                retime.source_frames(rows(timestamps))
        with self.assertRaises(ValueError):
            retime.source_frames([{"frame": 1, "tissue_pts_ms": 50}])

    def test_irregular_source_times_and_frame_pixels_survive_encoding(self):
        metadata = rows([None, 50, 83, 83, 150, 218])
        colors = [(i * 40, 80, 200 - i * 30) for i in range(len(metadata))]
        with tempfile.TemporaryDirectory(prefix="tissue-timing-test-") as temporary:
            video = Path(temporary) / "view.mp4"
            output = Path(temporary) / "source-time.mp4"
            subprocess.run([
                retime.FFMPEG, "-v", "error", "-f", "rawvideo", "-pixel_format", "rgb24",
                "-video_size", "16x16", "-framerate", "30", "-i", "-", "-an",
                "-c:v", "libx264", "-crf", "0", "-pix_fmt", "yuv444p", str(video),
            ], input=b"".join(bytes(color) * 256 for color in colors), check=True)
            selected = retime.source_frames(metadata)
            retime.encode(video, metadata, selected, output)
            self.assertEqual(retime.probe_timestamps(output), [0, 33, 100, 168])
            info = subprocess.run([
                retime.FFPROBE, "-v", "error", "-show_entries", "stream=duration,nb_frames",
                "-of", "json", str(output),
            ], check=True, capture_output=True, text=True)
            stream = json.loads(info.stdout)["streams"][0]
            self.assertEqual(int(stream["nb_frames"]), 4)
            self.assertAlmostEqual(float(stream["duration"]), 0.169, places=6)
            decoded = subprocess.run([
                retime.FFMPEG, "-v", "error", "-i", str(output), "-f", "rawvideo",
                "-pixel_format", "rgb24", "-pix_fmt", "rgb24", "-fps_mode", "passthrough", "-",
            ], check=True, capture_output=True).stdout
            self.assertEqual(len(decoded), 4 * 256 * 3)
            for index, source in enumerate([1, 2, 4, 5]):
                actual = decoded[index * 256 * 3:index * 256 * 3 + 3]
                for value, expected in zip(actual, colors[source]):
                    self.assertLessEqual(abs(value - expected), 4)


if __name__ == "__main__":
    unittest.main()
