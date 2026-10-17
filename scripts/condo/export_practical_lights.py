"""Export authored lamp placement without rebuilding meshes or lighting atlases."""
import os
from pathlib import Path
import sys

import bpy

sys.path.insert(0, str(Path(__file__).parent))
from bedroom_furniture import ROOT, bedroom_lights
from workspace_furniture import workspace_lights
from bathroom_furniture import bathroom_lights
from scene_export import update_manifest_entries


def export():
    for suffix in ('', '-lightmap'):
        bpy.ops.wm.open_mainfile(filepath=str(ROOT / f'toronto-condo{suffix}.blend'))
        lights = bedroom_lights()
        if 'Workspace detail lamp diffuser' in bpy.data.objects:
            lights.extend(workspace_lights())
        if 'Bathroom detail ceiling diffuser' in bpy.data.objects:
            lights.extend(bathroom_lights())
        filename = f'scene{suffix}.json'
        update_manifest_entries(ROOT, 'point_lights', lights, filenames=(filename,))
        print('PRACTICAL_LIGHTS_EXPORTED', filename, lights, flush=True)


if __name__ == '__main__':
    export()
    os._exit(0)
