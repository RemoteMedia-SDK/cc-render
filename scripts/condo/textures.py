"""Generate small, original, deterministic PBR textures for the condo asset pack."""
from pathlib import Path
import numpy as np
from PIL import Image
from facades import generate_facades

OUT = Path(__file__).resolve().parents[2] / "scenes/toronto_condo"
OUT.mkdir(parents=True, exist_ok=True)
rng = np.random.default_rng(731)
y, x = np.mgrid[0:512, 0:512] / 512

def save(name, color):
    Image.fromarray(np.uint8(np.clip(color, 0, 1)*255)).save(OUT/name)

grain = .018*np.sin(2*np.pi*(x*47+.5*np.sin(y*7)))+.008*rng.normal(size=x.shape)
oak = np.array([.69,.56,.40])+grain[:,:,None]
save('oak.png', oak)
vein = .025*np.sin(2*np.pi*(x*3+y*2+.2*np.sin(y*15)))+.008*rng.normal(size=x.shape)
save('stone.png',np.array([.76,.73,.66])+vein[:,:,None])
weave = .015*(np.sin(x*512*np.pi)+np.sin(y*512*np.pi))+.012*rng.normal(size=x.shape)
save('fabric.png',np.array([.78,.74,.67])+weave[:,:,None])
facade = np.full((512,512,3), [.40,.46,.49])
frame = (x%0.5<.04)|(y%0.5<.06)
facade[frame] = [.64,.65,.64]
facade[~frame] += (.04*np.sin(y[~frame]*8))[:,None]
save('facade.png',facade)
generate_facades(OUT)
dx=np.zeros_like(x)
dy=np.zeros_like(y)
# Periodic, seeded wave spectrum: many directions and wavelengths avoid a
# conspicuous two-sine interference pattern. Integer frequencies tile exactly.
for _ in range(80):
    kx,ky=rng.integers(-45,46,size=2)
    frequency=max(float(np.hypot(kx,ky)),1)
    phase=2*np.pi*(kx*x+ky*y)+rng.uniform(0,2*np.pi)
    slope=.045/np.sqrt(frequency/8)
    dx+=slope*kx/frequency*np.cos(phase)
    dy+=slope*ky/frequency*np.cos(phase)
normal=np.stack([-dx,-dy,np.ones_like(x)],axis=-1)
normal/=np.linalg.norm(normal,axis=-1)[:,:,None]
save('water_normal.png',normal*.5+.5)
print('Created original 512px surface textures and four facade finishes')
