// What the release tests read in docs/dev/infrastructure.md.
import fs from 'node:fs';
import path from 'node:path';

// infrastructureSection <root> <title>: the text of the section `## <title>` of
// docs/dev/infrastructure.md in the repository at <root>, '' when it has none.
export function infrastructureSection(root, title) {
  const docs = fs.readFileSync(path.join(root, 'docs/dev/infrastructure.md'), 'utf8');
  return docs.match(new RegExp(`^## ${title}\\n([\\s\\S]*?)(?=^## |(?![\\s\\S]))`, 'm'))?.[1] ?? '';
}

// pinsAttestation <line>: a gh attestation verify command in <line> pins the repository
// skald-so/skald, the release workflow and the ref of a release tag, refs/tags/v….
const workflowIdentity = 'skald-so/skald/\\.github/workflows/release\\.yml';
export const pinsAttestation = (line) =>
  /(--repo|-R)[\s=]+['"]?skald-so\/skald(?![\w/.-])/.test(line) &&
  ((new RegExp(`--signer-workflow[\\s=]+['"]?(https://)?(github\\.com/)?${workflowIdentity}`).test(line) &&
    /--source-ref[\s=]+['"]?refs\/tags\/v/.test(line)) ||
    new RegExp(`--cert-identity[\\s=]+['"]?https://github\\.com/${workflowIdentity}@refs/tags/v`).test(line));
