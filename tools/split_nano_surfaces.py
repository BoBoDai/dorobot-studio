#!/usr/bin/env python3
"""Separate Nano's existing STL surfaces for independent display materials.

Run with: uv run --no-project --with trimesh --with scipy tools/split_nano_surfaces.py
No tessellation, repair, simplification or coordinate changes are performed.
The output partitions the original binary STL triangle records exactly.
"""
import hashlib
import json
import struct
from pathlib import Path

import numpy as np
import trimesh


def main():
    folder = Path(__file__).resolve().parents[1] / "data/adora_nano/meshes/visual"
    source = folder / "base_link.stl"
    raw = source.read_bytes()
    count = struct.unpack_from("<I", raw, 80)[0]
    assert len(raw) == 84 + count * 50
    records = np.frombuffer(raw, dtype=np.uint8, offset=84).reshape(count, 50)
    mesh = trimesh.load_mesh(source, process=False)
    original_triangles = mesh.triangles.copy()
    original_normals = mesh.face_normals.copy()
    # Weld a temporary topology view only, to identify connected CAD pieces.
    # All selections and exported records still use the original coordinates.
    mesh.merge_vertices()
    np.testing.assert_allclose(mesh.triangles, original_triangles, atol=1e-8, rtol=0)
    components = trimesh.graph.connected_components(
        mesh.face_adjacency, nodes=np.arange(count), min_len=1
    )

    def component_with_bounds(lower, upper, faces):
        candidates = []
        for ids in components:
            if len(ids) != faces:
                continue
            points = original_triangles[ids].reshape(-1, 3)
            if np.allclose(points.min(0), lower, atol=2e-6) and np.allclose(
                points.max(0), upper, atol=2e-6
            ):
                candidates.append(ids)
        assert len(candidates) == 1, (lower, upper, len(candidates))
        return candidates[0]

    top_plate = component_with_bounds(
        [-.165155604, -.291500002, .197500005],
        [.165155604, .291513622, .209720463], 23764,
    )
    cover_shell = component_with_bounds(
        [-.184655771, -.308061987, .183862880],
        [.184655771, .307940245, .211085290], 19024,
    )
    screen = component_with_bounds(
        [-.179350004, -.140000001, .297699988],
        [-.010650001, .140000001, .337399989], 1052,
    )
    # The CAD already contains the inset display plane, bounded by its bezel.
    # Its two triangles are retained verbatim, without adding an overlay.
    screen_surface = screen[
        np.all(np.abs(original_triangles[screen, :, 2] - .3364) < 1e-6, axis=1)
        & (original_normals[screen, 2] > .999)
    ]
    assert len(screen_surface) == 2
    cover = np.concatenate([top_plate, cover_shell])
    bezel = np.setdiff1d(screen, screen_surface)
    body = np.setdiff1d(np.arange(count), np.concatenate([cover, screen]))
    groups = {"base_body": body, "base_cover": cover,
              "screen_frame": bezel, "screen_surface": screen_surface}
    np.testing.assert_array_equal(
        np.sort(np.concatenate(list(groups.values()))), np.arange(count)
    )
    report = {"source": source.name, "source_sha256": hashlib.sha256(raw).hexdigest(),
              "original_triangles": count, "triangle_records_preserved": True, "parts": {}}
    for name, ids in groups.items():
        ids = np.sort(ids)
        out = folder / f"{name}.stl"
        data = (f"Adora Nano original surfaces: {name}".encode().ljust(80, b"\0")
                + struct.pack("<I", len(ids)) + records[ids].tobytes())
        out.write_bytes(data)
        assert out.read_bytes()[84:] == records[ids].tobytes()
        report["parts"][name] = {"triangles": len(ids), "sha256": hashlib.sha256(data).hexdigest()}
    (folder / "surface-parts.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
