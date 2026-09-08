import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest
import zlib

import assets


class AssetsTests(unittest.TestCase):
    def test_gradient_is_a_valid_png_with_expected_pixels(self):
        image, digest = assets.gradient_png(2, 2)
        self.assertTrue(image.startswith(assets.PNG_SIGNATURE))
        offset = len(assets.PNG_SIGNATURE)
        chunks = {}
        while offset < len(image):
            size = struct.unpack_from(">I", image, offset)[0]
            kind = image[offset + 4:offset + 8]
            data = image[offset + 8:offset + 8 + size]
            checksum = struct.unpack_from(">I", image, offset + 8 + size)[0]
            self.assertEqual(checksum, zlib.crc32(kind + data))
            chunks[kind] = data
            offset += size + 12
        self.assertEqual(struct.unpack(">IIBBBBB", chunks[b"IHDR"]), (2, 2, 8, 2, 0, 0, 0))
        first = bytes([0, 0, 0, 255, 0, 127])
        second = bytes([0, 255, 127, 255, 255, 255])
        self.assertEqual(zlib.decompress(chunks[b"IDAT"]), b"\0" + first + b"\0" + second)
        self.assertEqual(digest, hashlib.sha256(first + second).hexdigest())
        self.assertEqual(assets.gradient_png(2, 2), (image, digest))

    def test_isolation_replaces_random_photos_and_records_rom_fingerprints(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            installed = root / "installed/cocovm/assets"
            (installed / "roms").mkdir(parents=True)
            (installed / "images").mkdir()
            (installed / "roms/coco3.rom").write_bytes(b"test fixture")
            (installed / "images/random.png").write_bytes(b"unused")
            fixture = root / "fixture"
            fixture.mkdir()
            output = root / "output"
            output.mkdir()
            assets.prepare("native", {"XDG_DATA_HOME": str(root / "installed")},
                           fixture, output / "metrics.json")
            isolated = fixture / "data/cocovm/assets"
            self.assertFalse(isolated.is_symlink())
            self.assertEqual((isolated / "roms").resolve(), (installed / "roms").resolve())
            self.assertEqual([path.name for path in (isolated / "images").iterdir()],
                             [assets.IMAGE_FILENAME])
            metadata = json.loads((output / "inputs.json").read_text())
            self.assertEqual(metadata["roms"][0]["sha256"], hashlib.sha256(b"test fixture").hexdigest())
            self.assertEqual(metadata["manager_image"]["width"], assets.IMAGE_WIDTH)
            self.assertEqual(metadata["manager_image"]["sha256"],
                             assets.fingerprint(isolated / "images" / assets.IMAGE_FILENAME)["sha256"])


if __name__ == "__main__":
    unittest.main()
