import { createHash } from 'node:crypto';
import { readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const manifestPath = join(root, 'release-manifest.json');
const packageInfo = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
const artifacts = ['app/coxeter_mfv.html', 'app/coxeter-mfv.exe'];
// The application source that a promotion copies from the working repository. Documentation, examples, data and
// the paper analyses are maintained here and stay out of the source digest.
const sourceEntries = [
  'assets',
  'docs/gut9-map',
  'index.html',
  'models',
  'package-lock.json',
  'package.json',
  'rust',
  'src',
  'test',
  'tools',
];
const ignoredDirectories = new Set(['node_modules', 'target', '.cache', '.git', '__pycache__']);
const textExtensions = /\.(?:js|mjs|json|md|toml|lock|rs|html|css|csv|txt|xml|sh|py|m|yml)$/;
const sourceFiles = [];

async function collect(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name, 'en'));
  for (const entry of entries) {
    const path = join(directory, entry.name);
    const relativePath = relative(root, path).replaceAll('\\', '/');
    if (relativePath === 'rust/fixtures/large') {
      continue;
    }
    if (entry.isDirectory()) {
      if (!ignoredDirectories.has(entry.name)) {
        await collect(path);
      }
    } else if (entry.isFile()) {
      sourceFiles.push(relativePath);
    }
  }
}

for (const entry of sourceEntries) {
  const path = join(root, entry);
  const info = await stat(path);
  if (info.isDirectory()) {
    await collect(path);
  } else {
    sourceFiles.push(entry);
  }
}
sourceFiles.sort();
const sourceHash = createHash('sha256');
for (const name of sourceFiles) {
  let bytes = await readFile(join(root, name));
  if (textExtensions.test(name) || name.endsWith('/.gitignore')) {
    bytes = Buffer.from(bytes.toString('utf8').replaceAll('\r\n', '\n'));
  }
  sourceHash.update(name);
  sourceHash.update('\0');
  sourceHash.update(bytes);
  sourceHash.update('\0');
}

const source = {
  sha256: sourceHash.digest('hex'),
  files: sourceFiles.length,
  normalization: 'UTF-8 text line endings normalized to LF; paths sorted; NUL-separated path and content',
};
const applications = [];
for (const path of artifacts) {
  const bytes = await readFile(join(root, path));
  applications.push({
    path,
    bytes: bytes.length,
    sha256: createHash('sha256').update(bytes).digest('hex'),
  });
}

const args = process.argv.slice(2);
if (args.includes('--write')) {
  const argument = args.find((value) => value.startsWith('--source-commit='));
  const commit = argument?.slice('--source-commit='.length);
  if (!commit || !/^[0-9a-f]{40}$/.test(commit)) {
    throw new Error('Writing requires --source-commit=<40-character working repository commit>.');
  }
  const manifest = {
    schemaVersion: 1,
    version: packageInfo.version,
    status: 'pre-publication',
    sourcePromotion: {
      repository: 'EscherMultiCellMultiCompartment',
      commit,
      scope: 'Application source, tests, assets and model inputs; official packaging and documentation maintained here',
    },
    source,
    applications,
  };
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log('Recorded source digest and application checksums in release-manifest.json.');
} else {
  const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  if (manifest.version !== packageInfo.version) {
    throw new Error('Manifest version differs from package.json.');
  }
  if (JSON.stringify(manifest.source) !== JSON.stringify(source)) {
    throw new Error('Distributed source differs from release-manifest.json. Review and rebuild before recording a new manifest.');
  }
  if (JSON.stringify(manifest.applications) !== JSON.stringify(applications)) {
    throw new Error('Application files differ from release-manifest.json.');
  }
  const citation = await readFile(join(root, 'CITATION.cff'), 'utf8');
  const packageLock = JSON.parse(await readFile(join(root, 'package-lock.json'), 'utf8'));
  const rust = await readFile(join(root, 'rust/Cargo.toml'), 'utf8');
  const nativeConfig = await readFile(join(root, 'rust/.cargo/config.toml'), 'utf8');
  if (packageLock.version !== packageInfo.version || packageLock.packages[''].version !== packageInfo.version) {
    throw new Error('npm lockfile version differs from the JavaScript package.');
  }
  if (!citation.includes(`version: ${packageInfo.version}`)) {
    throw new Error('CITATION.cff version differs from the JavaScript package.');
  }
  if (!rust.includes(`version = "${packageInfo.version}"`)) {
    throw new Error('Rust workspace version differs from the JavaScript package.');
  }
  if (!nativeConfig.includes(`COXETER_BUILD_VERSION = "${packageInfo.version}"`)) {
    throw new Error('Native provenance version differs from the JavaScript package.');
  }
  console.log(`Verified ${source.files} source files and both applications for version ${manifest.version}.`);
}
