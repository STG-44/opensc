#!/usr/bin/env python3
"""Export a north-up, metre-projected GeoTIFF for the OpenSC terrain playtest."""

import argparse
import math
from pathlib import Path
import struct

import numpy as np
import rasterio


def export(source_path, output, spacing, height_origin, spawn):
    if not 0.25 <= spacing <= 64.0 or not math.isfinite(height_origin):
        raise ValueError("Invalid sample spacing or elevation origin")
    if source_path.resolve() == output.resolve():
        raise ValueError("The input GeoTIFF and output heightfield must be different files")

    with rasterio.open(source_path) as source:
        if source.count != 1 or source.crs is None or not source.crs.is_projected:
            raise ValueError("Expected a one-band raster in a projected coordinate system")
        if not math.isclose(source.crs.linear_units_factor[1], 1.0, abs_tol=1e-9):
            raise ValueError("The coordinate system must use metres")
        transform = source.transform
        if transform.a <= 0 or transform.e >= 0 or transform.b != 0 or transform.d != 0:
            raise ValueError("Expected an unrotated north-up raster")
        east = source.bounds.right - source.bounds.left
        north = source.bounds.top - source.bounds.bottom
        width, height = round(east / spacing), round(north / spacing)
        if (
            not 2 <= width <= 2048
            or not 2 <= height <= 2048
            or width * height > 1_048_576
            or not math.isclose(width * spacing, east, abs_tol=1e-6)
            or not math.isclose(height * spacing, north, abs_tol=1e-6)
        ):
            raise ValueError("Sample spacing must divide both extents and fit the grid limits")
        if not (0 <= spawn[0] <= east and 0 <= spawn[1] <= north):
            raise ValueError("Spawn lies outside the raster")
        data = source.read(1, masked=True)
        if data.mask.any() or not np.isfinite(data).all():
            raise ValueError("Terrain contains missing elevation samples")

    # GeoTIFF stores north-to-south pixel centers. The runtime uses south-to-north
    # vertices and repeats its last sample along the outer edge of each axis.
    x_centers = (np.arange(data.shape[1]) + 0.5) * transform.a
    y_centers = (np.arange(data.shape[0]) + 0.5) * -transform.e
    x_vertices = np.arange(width) * spacing
    y_vertices = np.arange(height) * spacing
    east_rows = np.array([np.interp(x_vertices, x_centers, row) for row in data.data[::-1]])
    elevations = np.array(
        [np.interp(y_vertices, y_centers, column) for column in east_rows.T]
    ).T - height_origin
    if np.abs(elevations).max() > 10_000:
        raise ValueError("Elevations exceed the supported range relative to the selected origin")
    header = b"OpenSC heightfield 1\n" + struct.pack("<IIfff", width, height, spacing, *spawn)
    output.write_bytes(header + elevations.astype("<f4").tobytes())
    print(f"{output}: {width} × {height} samples, {east:g} × {north:g} m")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("geotiff", type=Path)
    parser.add_argument("heightfield", type=Path)
    parser.add_argument("--spacing", type=float, required=True, help="Sample spacing in metres")
    parser.add_argument("--height-origin", type=float, required=True, help="Elevation to subtract")
    parser.add_argument("--spawn", type=float, nargs=2, required=True, metavar=("EAST", "NORTH"))
    args = parser.parse_args()
    try:
        export(args.geotiff, args.heightfield, args.spacing, args.height_origin, args.spawn)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
