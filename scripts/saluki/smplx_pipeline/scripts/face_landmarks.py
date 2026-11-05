"""Run MediaPipe FaceLandmarker on face renders; save pixel landmarks + an overlay image.

usage: mpenv/python face_landmarks.py <model.task> <png> [<png> ...]
Writes <png>.lm.json ({"uv": [[u, v], ...] in pixels, 478 points}) and <png>.lm.png.
"""
import json
import sys

import cv2
import mediapipe as mp
from mediapipe.tasks.python import BaseOptions
from mediapipe.tasks.python.vision import FaceLandmarker, FaceLandmarkerOptions

model, pngs = sys.argv[1], sys.argv[2:]
opts = FaceLandmarkerOptions(base_options=BaseOptions(model_asset_path=model), num_faces=1,
                             output_face_blendshapes=False)
with FaceLandmarker.create_from_options(opts) as lm:
    for p in pngs:
        img = cv2.imread(p)
        h, w = img.shape[:2]
        res = lm.detect(mp.Image(image_format=mp.ImageFormat.SRGB, data=cv2.cvtColor(img, cv2.COLOR_BGR2RGB)))
        if not res.face_landmarks:
            print("NO FACE", p)
            continue
        uv = [[q.x * w, q.y * h] for q in res.face_landmarks[0]]
        json.dump({"uv": uv}, open(p + ".lm.json", "w"))
        for u, v in uv:
            cv2.circle(img, (int(u), int(v)), 2, (0, 255, 0), -1)
        cv2.imwrite(p + ".lm.png", img)
        print("OK", p, len(uv))
