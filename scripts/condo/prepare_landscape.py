"""Prepare scenic vegetation inside Toronto park polygons, never across the airport.

The park boundaries are source data; tree positions and sizes are deterministic
art direction, not a tree inventory. Requires Shapely and PyProj.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import random

from shapely.geometry import Point, Polygon, box, shape
from shapely.ops import transform, unary_union
from shapely.strtree import STRtree

from geography import OUT, ORIGIN, project, triangles


def building_footprints(buildings):
    footprints = []
    for building in buildings:
        ends = building['parts'] + [len(building['xy'])]
        for start, end in zip(ends, ends[1:]):
            ring = building['xy'][start:end]
            if len(ring) >= 3:
                footprints.append(Polygon(ring).buffer(0))
    return STRtree(footprints)


def prepare(source):
    features = json.loads(source.read_text())['features']
    crop = box(-2800, -3500, 3500, 1100)
    selected = [f for f in features if f['properties']['AREA_CLASS'] in ('Park', 'Open Green Space')]
    parks = unary_union([transform(project, shape(f['geometry'])).buffer(0) for f in selected]).intersection(crop)
    geography = json.loads((OUT/'geography.json').read_text())
    land = unary_union([Polygon(t) for part in geography['land'] for t in part['triangles']])
    parks = parks.intersection(land).simplify(.6, preserve_topology=True)
    planting = parks.buffer(-12)
    buildings = building_footprints(json.loads((OUT/'massing.json').read_text())['buildings'])
    roads = STRtree([Polygon(t) for road in geography['roads'] for t in road['triangles']])
    rng = random.Random(731)
    trees = []
    for x in range(-2800, 3500, 24):
        for y in range(-3500, 1100, 24):
            px, py = x+rng.uniform(-8,8), y+rng.uniform(-8,8)
            # Groves and open lawns, rather than a regular plantation grid.
            density = math.sin(px/83) + math.sin(py/67) + .6*math.sin((px+py)/37)
            point = Point(px,py)
            if density < -.4 or not planting.contains(point):
                continue
            if len(buildings.query(point, predicate='dwithin', distance=9)) or len(roads.query(point, predicate='dwithin', distance=8)):
                continue
            trees.append([round(px,2), round(py,2), round(rng.uniform(7,14),2), rng.randrange(3)])
    result = {'origin':ORIGIN, 'seed':731, 'parks':triangles(parks), 'trees':trees,
              'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
              'note':'Mapped park extents, scenic tree positions; 12 m park/shore margin, 9 m building and 8 m road clearance.'}
    (OUT/'landscape.json').write_text(json.dumps(result,separators=(',',':'))+'\n')
    print(json.dumps({'park_features':len(selected),'park_area_m2':round(parks.area),
                      'park_triangles':len(result['parks']),'trees':len(trees)}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('parks',type=Path)
    prepare(parser.parse_args().parks)
