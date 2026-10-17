"""Original scenic facade textures; not a survey of individual buildings."""
import numpy as np
from PIL import Image


def generate_facades(output):
    y, x = np.mgrid[0:512, 0:512]/512
    column, row = (x*4).astype(int), (y*4).astype(int)
    u, v = (x*4)%1, (y*4)%1
    palettes = [
        ((.23,.36,.39), (.46,.51,.51), .055, .09, .25),
        ((.20,.27,.34), (.34,.38,.40), .045, .075, .30),
        ((.25,.33,.34), (.66,.62,.53), .18, .20, .36),
        ((.20,.28,.30), (.43,.28,.20), .20, .23, .40),
    ]
    rng = np.random.default_rng(731)
    for index,(glazing,frame_color,frame_x,frame_y,roughness) in enumerate(palettes):
        frame = (u<frame_x)|(v<frame_y)
        variation = rng.uniform(-.045,.045,(4,4))[row,column]
        color = np.array(glazing)[None,None,:]+variation[:,:,None]
        color[frame] = frame_color
        # Restrained dark reveals along the lower and left window edges.
        reveal = (~frame)&((u<frame_x+.025)|(v<frame_y+.025))
        color[reveal] *= .65
        Image.fromarray(np.uint8(np.clip(color,0,1)*255)).save(output/f'facade-{index}.png')
        surface = np.where(frame,.82,roughness)
        Image.fromarray(np.uint8(surface*255)).save(output/f'facade-{index}-rough.png')


if __name__ == '__main__':
    from pathlib import Path
    generate_facades(Path(__file__).resolve().parents[2]/'scenes/toronto_condo')
