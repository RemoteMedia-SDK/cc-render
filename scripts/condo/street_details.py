"""Scenic lane paint and barriers following the existing surveyed road paths."""
import math
from mathutils import Vector
from landscape import mesh_object


def segments(path):
    result = []
    for first, last in zip(path, path[1:]):
        a, b = Vector((first[0], -first[2])), Vector((last[0], -last[2]))
        delta = b-a
        result.append((a, delta.normalized(), delta.length))
    return result


def point_at(road, distance):
    for a, direction, length in road:
        if distance <= length:
            return a + direction*distance, Vector((-direction.y, direction.x))
        distance -= length
    a, direction, length = road[-1]
    return a+direction*length, Vector((-direction.y, direction.x))


def strip(vertices, faces, road, start, end, offset, width, height):
    a, na = point_at(road, start)
    b, nb = point_at(road, end)
    base = len(vertices)
    for point, normal, edge in [(a,na,-1),(b,nb,-1),(b,nb,1),(a,na,1)]:
        p = point + normal*(offset + edge*width/2)
        vertices.append((p.x,p.y,height))
    faces.append(tuple(base+i for i in range(4)))


def barrier(vertices, faces, a, direction, length, offset, height):
    normal = Vector((-direction.y,direction.x))
    base = len(vertices)
    for z in [height, height+.75]:
        for point, edge in [(a,-1),(a+direction*length,-1),(a+direction*length,1),(a,1)]:
            p = point+normal*(offset+edge*.14)
            vertices.append((p.x,p.y,z))
    faces.extend(tuple(base+i for i in f) for f in [(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)])


def build_street_details(geography, collection, material):
    paint_vertices, paint_faces, rail_vertices, rail_faces = [],[],[],[]
    carriageways = 0
    for lane in geography['traffic']:
        if lane['start'][1] < -85:
            continue
        road = segments(lane.get('path') or [lane['start'],lane['end']])
        length = sum(segment[2] for segment in road)
        elevation = lane['start'][1]-.03
        for offset in [-1.75,1.75]:
            for distance in range(0,math.floor(length),12):
                strip(paint_vertices,paint_faces,road,distance,min(distance+4,length),offset,.18,elevation)
        for offset in [-5.25,5.25]:
            cursor = 0
            for a,direction,span in road:
                strip(paint_vertices,paint_faces,road,cursor,cursor+span,offset,.18,elevation)
                barrier(rail_vertices,rail_faces,a,direction,span,math.copysign(9.2,offset),elevation-.02)
                cursor += span
        carriageways += 1
    mesh_object('Gardiner lane paint',paint_vertices,paint_faces,material('Road paint',(.72,.72,.66),.85),collection)
    mesh_object('Gardiner safety barriers',rail_vertices,rail_faces,material('Road barrier concrete',(.46,.45,.41),.9),collection)
    print('STREET_DETAILS',carriageways,'carriageways',len(paint_faces),'paint strips',flush=True)
