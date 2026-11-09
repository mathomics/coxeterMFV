import json
import hashlib
from pathlib import Path
import subprocess


root = Path(__file__).resolve().parent.parent
command = [
    'cargo',
    'metadata',
    '--locked',
    '--format-version',
    '1',
    '--filter-platform',
    'x86_64-pc-windows-msvc',
]
result = subprocess.run(command, cwd=root / 'rust', check=True, capture_output=True, encoding='utf-8')
metadata = json.loads(result.stdout)
packages = sorted(metadata['packages'], key=lambda package: (package['name'], package['version']))
upstream_sources = json.loads((root / 'LICENSES/native/sources.json').read_text(encoding='utf-8'))
lines = [
    'Third-party dependency notices for the Windows distribution',
    '',
    'Generated from rust/Cargo.lock and downloaded Cargo package sources.',
    'This includes build dependencies and packages in the resolved graph; inclusion',
    'does not imply that every package is linked into the executable.',
    'Project source is covered separately by the root MIT LICENSE.',
    '',
]
missing = []
count = 0
for package in packages:
    if package['source'] is None:
        continue
    folder = Path(package['manifest_path']).parent
    licences = []
    for path in folder.rglob('*'):
        if not path.is_file():
            continue
        name = path.name.upper()
        if name.startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'UNLICENSE')):
            licences.append(path)
    licence_file = package.get('license_file')
    if licence_file:
        path = folder / licence_file
        if path.is_file() and path not in licences:
            licences.append(path)
    upstream_files = []
    package_label = package['name'] + ' ' + package['version']
    for group in upstream_sources:
        if package_label in group['packages']:
            upstream_files.extend(group['files'])
    lines.append(f"{package['name']} {package['version']}")
    lines.append(f"SPDX expression: {package.get('license') or 'See licence files'}")
    lines.append(f"Source: https://crates.io/crates/{package['name']}/{package['version']}")
    if package.get('repository'):
        lines.append(f"Upstream: {package['repository']}")
    if not licences and not upstream_files:
        missing.append(f"{package['name']} {package['version']}")
        lines.append('No licence text found in the published package; consult upstream.')
    for path in sorted(licences):
        lines.append('')
        lines.append(f"File: {path.relative_to(folder).as_posix()}")
        lines.append(path.read_text(encoding='utf-8', errors='replace').strip())
    for entry in upstream_files:
        path = root / 'LICENSES/native' / entry['path']
        data = path.read_bytes()
        if hashlib.sha256(data).hexdigest() != entry['sha256']:
            raise ValueError(f'Upstream licence checksum mismatch: {entry["path"]}')
        lines.append('')
        lines.append(f"Source: {entry['source']}")
        lines.append(data.decode('utf-8').strip())
    lines.append('')
    count += 1

output = root / 'app/THIRD_PARTY_LICENSES.txt'
output.write_text('\n'.join(lines), encoding='utf-8', newline='\n')
print(f'Recorded notices for {count} dependency packages in {output.relative_to(root)}.')
if missing:
    print('Packages requiring upstream licence-text review:')
    for name in missing:
        print(name)
