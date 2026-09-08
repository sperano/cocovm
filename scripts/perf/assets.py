"""Deterministic, redistributable manager image and installed-ROM fingerprints."""
import hashlib
import json
from pathlib import Path
import struct
import zlib

IMAGE_WIDTH = 1280
IMAGE_HEIGHT = 960
IMAGE_FILENAME = "manager-gradient.png"
GENERATOR_VERSION = "rgb-gradient-v1"
CHANNEL_MAX = 255
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
PNG_BIT_DEPTH = 8
PNG_RGB_COLOR_TYPE = 2
PNG_STANDARD_METHOD = 0
PNG_FILTER_NONE = b"\0"
COMPRESSION_LEVEL = 9
HASH_CHUNK_BYTES = 65536
COCO3_ROM = "coco3.rom"


def fingerprint(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(HASH_CHUNK_BYTES):
            digest.update(chunk)
    return {"name": path.name, "bytes": path.stat().st_size, "sha256": digest.hexdigest()}


def png_chunk(kind, payload):
    return (struct.pack(">I", len(payload)) + kind + payload
            + struct.pack(">I", zlib.crc32(kind + payload)))


def gradient_png(width=IMAGE_WIDTH, height=IMAGE_HEIGHT):
    rows = bytearray()
    pixels = hashlib.sha256()
    for y in range(height):
        row = bytes(channel for x in range(width) for channel in (
            x * CHANNEL_MAX // max(width - 1, 1),
            y * CHANNEL_MAX // max(height - 1, 1),
            (x + y) * CHANNEL_MAX // max(width + height - 2, 1)))
        pixels.update(row)
        rows.extend(PNG_FILTER_NONE)
        rows.extend(row)
    header = struct.pack(">IIBBBBB", width, height, PNG_BIT_DEPTH, PNG_RGB_COLOR_TYPE,
                         PNG_STANDARD_METHOD, PNG_STANDARD_METHOD, PNG_STANDARD_METHOD)
    image = (PNG_SIGNATURE + png_chunk(b"IHDR", header)
             + png_chunk(b"IDAT", zlib.compress(rows, COMPRESSION_LEVEL)) + png_chunk(b"IEND", b""))
    return image, pixels.hexdigest()


def source_directory(environment):
    return Path(environment.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "cocovm/assets"


def isolate_native(source, destination):
    destination.mkdir(parents=True)
    (destination / "roms").symlink_to((source / "roms").resolve(), target_is_directory=True)
    images = destination / "images"
    images.mkdir()
    image, pixels_sha256 = gradient_png()
    path = images / IMAGE_FILENAME
    path.write_bytes(image)
    return {**fingerprint(path), "generator": GENERATOR_VERSION,
            "width": IMAGE_WIDTH, "height": IMAGE_HEIGHT, "pixels_sha256": pixels_sha256,
            "zlib_version": zlib.ZLIB_RUNTIME_VERSION}


def prepare(kind, environment, fixture, report):
    source = source_directory(environment)
    result = {"roms": [fingerprint(path) for path in sorted((source / "roms").glob("*.rom"))
                       if path.is_file()]}
    if kind == "native":
        result["manager_image"] = isolate_native(source, fixture / "data/cocovm/assets")
    else:
        coco3 = source / "roms" / COCO3_ROM
        result["core_basic_rom"] = fingerprint(coco3) if coco3.is_file() else None
    (report.parent / "inputs.json").write_text(json.dumps(result, indent=2))
