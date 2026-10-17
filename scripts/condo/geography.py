"""Crop Toronto public waterbody/centreline GeoJSON into local scene geometry.

Input files are downloaded unchanged from the sources listed in provenance.json.
Requires shapely and pyproj; output is independent of those libraries at runtime.
"""
import argparse
from collections import defaultdict
import json
from pathlib import Path

from pyproj import CRS, Transformer
from shapely.geometry import box, shape, mapping
from shapely.ops import transform, unary_union, triangulate, linemerge

OUT = Path(__file__).resolve().parents[2] / "scenes/toronto_condo"
ORIGIN = [43.6409, -79.3971]
local = CRS.from_proj4(f"+proj=aeqd +lat_0={ORIGIN[0]} +lon_0={ORIGIN[1]} +datum=WGS84 +units=m")
project = Transformer.from_crs("EPSG:4326", local, always_xy=True).transform
extent = box(-6500, -6500, 6500, 5000)


def pieces(geometry):
    return list(geometry.geoms) if hasattr(geometry, "geoms") else [geometry]


def triangles(geometry):
    result = []
    for polygon in pieces(geometry):
        if polygon.geom_type != "Polygon":
            continue
        for triangle in triangulate(polygon):
            if polygon.covers(triangle):
                result.append([[round(x, 2), round(y, 2)] for x, y in list(triangle.exterior.coords)[:3]])
    return result


def prepare(water_path, road_path):
    water = json.loads(water_path.read_text())["features"]
    water = unary_union([transform(project, shape(f["geometry"])) for f in water]).intersection(extent)
    land = extent.difference(water).simplify(1.0, preserve_topology=True)
    land_parts = []
    for polygon in pieces(land):
        if polygon.area < 10:
            continue
        mesh = triangles(polygon)
        land_parts.append({"island": polygon.centroid.y < -700, "triangles": mesh,
                           "shore": [list(p) for p in polygon.exterior.coords]})
    roads = []
    routes = defaultdict(list)
    crop = box(-2800, -3300, 2800, 2200)
    for feature in json.loads(road_path.read_text())["features"]:
        props = feature["properties"]
        geo = shape(feature["geometry"])
        # Cheap lon/lat bound filter before projection.
        if not geo.intersects(box(-79.44, 43.60, -79.35, 43.665)):
            continue
        line = transform(project, geo).intersection(crop)
        name = props.get("LINEAR_NAME_FULL", "")
        kind = props.get("FEATURE_CODE_DESC", "")
        if line.is_empty or not any(word in kind.lower() for word in ("arterial", "local", "collector", "expressway", "laneway")):
            continue
        elevated = name in ("F G Gardiner Xy W", "F G Gardiner Xy E")
        width = 10 if elevated else 8 if "arterial" in kind.lower() else 4
        # Geometry follows surveyed centreline; widths/elevations are scenic approximations.
        z = -80.4 if elevated else -90.15
        road = line.buffer(width, cap_style=2, join_style=2).intersection(extent)
        roads.append({"name": name, "z": z, "triangles": triangles(road)})
        if name in ("F G Gardiner Xy W", "F G Gardiner Xy E", "Queens Quay W"):
            routes[name].extend(g for g in pieces(line) if g.geom_type == "LineString")
    traffic = []
    for name, lines in routes.items():
        merged = linemerge(unary_union(lines))
        for path in pieces(merged):
            if path.length < 600:
                continue
            points = list(path.simplify(.4).coords)
            westbound = name.endswith(" W") and name.startswith("F G")
            if (points[-1][0] < points[0][0]) != westbound:
                points.reverse()
            elevated = name.startswith("F G")
            z = -80.35 if elevated else -90.1
            runtime = [[round(x, 2), z, round(-y, 2)] for x, y in points]
            traffic.append({"start": runtime[0], "end": runtime[-1], "path": runtime,
                            "speed_mps": 21 if elevated else 10, "count": min(70, int(path.length/65))})
    result = {"origin": ORIGIN, "land": land_parts, "roads": roads, "traffic": traffic}
    (OUT / "geography.json").write_text(json.dumps(result, separators=(",", ":")))
    print("land parts", len(land_parts), "road segments", len(roads), "traffic routes", len(traffic))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("water", type=Path)
    parser.add_argument("roads", type=Path)
    args = parser.parse_args()
    prepare(args.water, args.roads)
