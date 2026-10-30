# SPDX-License-Identifier: MIT
"""Blender 4.x addon stub — Max-FX KF2/SKD/KFS importer (OpenMaxFX Phase 3)."""

bl_info = {
    "name": "Max-FX KF2/SKD/KFS (OpenMaxFX)",
    "author": "OpenMaxFX",
    "version": (0, 1, 0),
    "blender": (4, 0, 0),
    "location": "File > Import",
    "description": "Import Remedy Max-FX KF2/SKD/KFS (WIP)",
    "category": "Import-Export",
}

import bpy
from bpy_extras.io_utils import ImportHelper
from bpy.props import StringProperty
from pathlib import Path


def _extract_strings(data: bytes) -> list[str]:
    """Pull 0D 14 length-prefixed strings used across KF2/SKD/KFS."""
    out: list[str] = []
    i = 0
    while i + 3 < len(data):
        if data[i] == 0x0D and data[i + 1] == 0x14:
            n = data[i + 2]
            chunk = data[i + 3 : i + 3 + n]
            i += 3 + n
            if n and all(32 <= c < 127 for c in chunk):
                out.append(chunk.decode("ascii", "replace"))
            continue
        i += 1
    return out


class ImportKF2(bpy.types.Operator, ImportHelper):
    bl_idname = "import_scene.maxfx_kf2"
    bl_label = "Max-FX KF2/SKD/KFS"
    filename_ext = ".kf2"
    filter_glob: StringProperty(default="*.kf2;*.KF2;*.skd;*.SKD;*.kfs;*.KFS", options={"HIDDEN"})

    def execute(self, context):
        path = Path(self.filepath)
        data = path.read_bytes()
        strings = _extract_strings(data)
        # WIP: create empties named after bone/object strings as a visible scaffold
        for name in strings:
            if not name or name.startswith("textures"):
                continue
            if name in bpy.data.objects:
                continue
            empty = bpy.data.objects.new(name[:63], None)
            context.collection.objects.link(empty)
        self.report({"INFO"}, f"OpenMaxFX: {len(strings)} tags from {path.name} (mesh import WIP)")
        return {"FINISHED"}


def menu_func(self, context):
    self.layout.operator(ImportKF2.bl_idname, text="Max-FX KF2/SKD/KFS (.kf2/.skd/.kfs)")


def register():
    bpy.utils.register_class(ImportKF2)
    bpy.types.TOPBAR_MT_file_import.append(menu_func)


def unregister():
    bpy.types.TOPBAR_MT_file_import.remove(menu_func)
    bpy.utils.unregister_class(ImportKF2)


if __name__ == "__main__":
    register()
