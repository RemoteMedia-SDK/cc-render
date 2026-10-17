"""Deterministic scenic broadleaf trees for the near waterfront parks."""
import math
import random
from mathutils import Vector


def branch(batch, start, end, radius):
    vertices, faces = batch
    axis = (end-start).normalized()
    side = axis.cross(Vector((0, 1, 0))).normalized()
    up = axis.cross(side)
    base = len(vertices)
    for center, scale in [(start, 1), (end, .35)]:
        for i in range(6):
            angle = i*math.tau/6
            vertices.append(tuple(center+radius*scale*(side*math.cos(angle)+up*math.sin(angle))))
    faces.extend((base+i, base+(i+1)%6, base+(i+1)%6+6, base+i+6) for i in range(6))


def leaf(batch, center, rng):
    vertices, faces = batch
    angle = rng.uniform(0, math.tau)
    tilt = rng.uniform(-.7, .7)
    along = Vector((math.cos(angle), math.sin(angle), tilt)).normalized()*rng.uniform(.28, .38)
    across = Vector((-math.sin(angle), math.cos(angle), rng.uniform(-.5, .5)))*.19
    base = len(vertices)
    vertices.extend(tuple(p) for p in [center-along, center+across, center+along,
                                      center-across, center+Vector((0, 0, .035))])
    faces.extend((base+i, base+(i+1)%4, base+4) for i in range(4))


def leaf_cluster(batches, center, radius, rng):
    for _ in range(100):
        azimuth = rng.uniform(0, math.tau)
        vertical = rng.uniform(-1, 1)
        distance = rng.random()**(1/3)
        horizontal = math.sqrt(1-vertical*vertical)
        offset = Vector((horizontal*math.cos(azimuth), horizontal*math.sin(azimuth), vertical*.8))
        leaf(batches[rng.randrange(len(batches))], center+offset*radius*distance, rng)


def detailed_tree(x, y, height, index, leaves, wood):
    rng = random.Random(731+index*104729)
    root = Vector((x, y, -90.5))
    trunk_top = root+Vector((height*.025, 0, height*.88))
    branch(wood, root, trunk_top, height*.022)
    phase = rng.uniform(0, math.tau)
    for tier in range(3):
        for limb in range(5):
            angle = phase+limb*math.tau/5+tier*1.7
            spread = height*(.20-.025*tier)*rng.uniform(.85, 1.1)
            start = root+Vector((0, 0, height*(.30+.13*tier)))
            end = root+Vector((math.cos(angle)*spread, math.sin(angle)*spread,
                               height*(.54+.13*tier)+rng.uniform(-.25, .25)))
            branch(wood, start, end, height*.009)
            leaf_cluster(leaves, end, height*.12, rng)
    leaf_cluster(leaves, trunk_top, height*.12, rng)
