"""Render Nano's URDF visual meshes/materials without altering collision bodies."""

import hashlib
import math
import xml.etree.ElementTree as ET
from pathlib import Path

import numpy as np


def visual_specs(model):
    root = ET.parse(model).getroot()
    materials = {
        m.get("name"): [float(x) for x in m.find("color").get("rgba").split()]
        for m in root.findall("material")
        if m.find("color") is not None
    }
    specs = []
    for link in root.findall("link"):
        for visual in link.findall("visual"):
            mesh = visual.find("geometry/mesh")
            if mesh is None:
                raise ValueError("Nano visual renderer expects STL meshes")
            material = visual.find("material")
            rgba = [0.62, 0.65, 0.70, 1.0]
            if material is not None:
                color = material.find("color")
                rgba = (
                    [float(x) for x in color.get("rgba").split()]
                    if color is not None
                    else materials.get(material.get("name"), rgba)
                )
            origin = visual.find("origin")
            specs.append(
                dict(
                    link=link.get("name"),
                    rgba=rgba,
                    path=str((Path(model).parent / mesh.get("filename")).resolve()),
                    scale=[float(x) for x in mesh.get("scale", "1 1 1").split()],
                    xyz=[
                        float(x)
                        for x in (
                            origin.get("xyz", "0 0 0")
                            if origin is not None
                            else "0 0 0"
                        ).split()
                    ],
                    rpy=[
                        float(x)
                        for x in (
                            origin.get("rpy", "0 0 0")
                            if origin is not None
                            else "0 0 0"
                        ).split()
                    ],
                )
            )
    return specs


def axis_angle(rpy):
    r, p, y = np.asarray(rpy) / 2
    cr, cp, cy = np.cos([r, p, y])
    sr, sp, sy = np.sin([r, p, y])
    q = np.array(
        [
            sr * cp * cy - cr * sp * sy,
            cr * sp * cy + sr * cp * sy,
            cr * cp * sy - sr * sp * cy,
        ]
    )
    w = cr * cp * cy + sr * sp * sy
    length = float(np.linalg.norm(q))
    return (
        (q * (2 * math.atan2(length, w) / length)).tolist()
        if length > 1e-12
        else [0.0, 0.0, 0.0]
    )


def register_visuals(scene, native, model):
    nx = scene.nexus3d
    handles = dict(zip(native.link_names, native.link_body_handles, strict=True))
    specs = visual_specs(model)
    for spec in specs:
        data = Path(spec["path"]).read_bytes()
        count = int.from_bytes(data[80:84], "little")
        if len(data) != 84 + count * 50:
            raise ValueError(f"Expected binary STL: {spec['path']}")
        triangles = np.frombuffer(
            data,
            dtype=np.dtype([("n", "<f4", (3,)), ("v", "<f4", (3, 3)), ("attr", "<u2")]),
            offset=84,
        )["v"]
        vertices, indices = np.unique(
            (triangles * spec["scale"]).reshape(-1, 3), axis=0, return_inverse=True
        )
        faces = indices.reshape(-1, 3)
        faces = faces[
            (faces[:, 0] != faces[:, 1])
            & (faces[:, 1] != faces[:, 2])
            & (faces[:, 0] != faces[:, 2])
        ]
        shape = (
            nx.ColliderBuilder.trimesh(vertices.tolist(), faces.tolist())
            .build()
            .shared_shape()
        )
        scene.engine.viewer.insert_sensor_shape(
            0,
            handles[spec["link"]],
            shape,
            nx.Pose(nx.Vec3(*spec["xyz"]), nx.Vec3(*axis_angle(spec["rpy"]))),
            spec["rgba"],
        )
        spec["triangles"] = len(faces)
        spec["sha256"] = hashlib.sha256(data).hexdigest()
    return specs
