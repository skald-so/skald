// The WinGet manifest set for a stable Skald release.
const schema = '1.28.0';
const locale = 'en-US';
const project = 'https://github.com/skald-so/skald';
export const windowsTargets = {
  x64: 'x86_64-pc-windows-msvc',
  arm64: 'aarch64-pc-windows-msvc',
};

export const stableVersion = /^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)$/;
export const archiveName = (target) => `skald-client-${target}.zip`;
export const releaseUrl = (version) => `${project}/releases/download/v${version}`;

export function checkPackage(id, version) {
  if (id?.length > 128 || !/^[A-Za-z0-9][A-Za-z0-9_-]{0,31}(\.[A-Za-z0-9][A-Za-z0-9_-]{0,31}){1,7}$/.test(id)) {
    throw new Error('Expected a WinGet package identifier such as Publisher.Package.');
  }
  if (version?.length > 128 || !stableVersion.test(version)) throw new Error('Expected a stable release version.');
}

export function manifests(id, version, entries) {
  const header = (type) => `# yaml-language-server: $schema=https://aka.ms/winget-manifest.${type}.${schema}.schema.json
PackageIdentifier: ${JSON.stringify(id)}
PackageVersion: ${JSON.stringify(version)}
`;
  const footer = (type) => `ManifestType: ${type}
ManifestVersion: "${schema}"
`;
  return {
    [`${id}.yaml`]: header('version') + `DefaultLocale: ${locale}\n` + footer('version'),
    [`${id}.locale.${locale}.yaml`]: header('defaultLocale') + `PackageLocale: ${locale}
Publisher: Skald
PublisherUrl: https://skald.so
PackageName: skald
PackageUrl: ${project}
License: MIT OR Apache-2.0
LicenseUrl: ${project}/blob/v${version}/LICENSE.md
ShortDescription: Message broker and task queue for AI agents.
` + footer('defaultLocale'),
    [`${id}.installer.yaml`]: header('installer') + `InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
  - RelativeFilePath: skald.exe
    PortableCommandAlias: skald
Installers:
` + entries.map(({ architecture, url, sha256 }) => `  - Architecture: ${architecture}
    InstallerUrl: ${JSON.stringify(url)}
    InstallerSha256: "${sha256}"
`).join('') + footer('installer'),
  };
}
