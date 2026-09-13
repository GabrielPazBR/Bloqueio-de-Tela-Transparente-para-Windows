"""Verify package bytes and PE metadata without executing either binary.

This does not verify Authenticode trust; package.ps1 invokes SignTool for that.
"""
import hashlib
import json
from pathlib import Path
import struct
import sys
import xml.etree.ElementTree as ET


class PE:
    def __init__(self, data):
        self.data = data
        assert data[:2] == b'MZ', 'missing DOS header'
        self.header = struct.unpack_from('<I', data, 0x3C)[0]
        assert data[self.header:self.header + 4] == b'PE\0\0', 'missing PE header'
        self.machine, count = struct.unpack_from('<HH', data, self.header + 4)
        optional_size = struct.unpack_from('<H', data, self.header + 20)[0]
        optional = self.header + 24
        magic = struct.unpack_from('<H', data, optional)[0]
        self.directories = optional + (112 if magic == 0x20B else 96)
        self.sections = []
        for i in range(count):
            entry = optional + optional_size + 40 * i
            virtual_size, rva, raw_size, raw = struct.unpack_from('<IIII', data, entry + 8)
            self.sections.append((rva, max(virtual_size, raw_size), raw))

    def offset(self, rva):
        for start, size, raw in self.sections:
            if start <= rva < start + size:
                return raw + rva - start
        raise ValueError('RVA outside sections')

    def manifest(self):
        rva, _ = struct.unpack_from('<II', self.data, self.directories + 16)
        base = self.offset(rva)

        def entries(relative):
            named, ids = struct.unpack_from('<HH', self.data, base + relative + 12)
            for i in range(named + ids):
                yield struct.unpack_from('<II', self.data, base + relative + 16 + 8 * i)

        resource = next(value for key, value in entries(0) if key == 24)
        while resource & 0x80000000:
            _, resource = next(entries(resource & 0x7FFFFFFF))
        rva, size = struct.unpack_from('<II', self.data, base + resource)
        start = self.offset(rva)
        return ET.fromstring(self.data[start:start + size])


def verify(directory):
    metadata = json.loads((directory / 'package.json').read_text(encoding='utf-8-sig'))
    app = (directory / 'BloqueioTransparente.exe').read_bytes()
    setup = (directory / metadata['installer']).read_bytes()
    assert hashlib.sha256(app).hexdigest().upper() == metadata['applicationSha256']
    assert hashlib.sha256(setup).hexdigest().upper() == metadata['installerSha256']
    assert app != setup, 'installer must be a separate executable'
    assert app in setup, 'installer must embed the exact application, including its signature'
    app_pe, setup_pe = PE(app), PE(setup)
    expected = 0x8664 if metadata['architecture'] == 'x64' else 0x14C
    assert app_pe.machine == setup_pe.machine == expected
    manifest = setup_pe.manifest()
    level = manifest.find('.//{urn:schemas-microsoft-com:asm.v3}requestedExecutionLevel')
    assert level is not None and level.get('level') == 'requireAdministrator'
    assert level.get('uiAccess') == 'false'
    return {
        'version': metadata['version'], 'architecture': metadata['architecture'],
        'exactEmbeddedPayload': True, 'administratorManifest': True,
        'hashesMatch': True, 'signing': metadata['signing'],
    }


if __name__ == '__main__':
    print(json.dumps(verify(Path(sys.argv[1])), indent=2))
