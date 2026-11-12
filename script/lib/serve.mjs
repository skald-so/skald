// Serves the files of <dir> over HTTP on 127.0.0.1, as GitHub serves release files: each as
// application/octet-stream, 404 for any other path. Prints the port it listens on, a free one, then
// the path of each request, and runs until killed. For the installers' tests: script/installers-test.
// Usage: node script/lib/serve.mjs <dir>
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';

const dir = path.resolve(process.argv[2]);
const server = http.createServer((request, response) => {
  console.log(request.url);
  const name = decodeURIComponent(new URL(request.url, 'http://127.0.0.1').pathname.slice(1));
  const file = path.join(dir, name);
  if (request.method !== 'GET' || name.includes('/') || name.includes('\\') || !fs.statSync(file, { throwIfNoEntry: false })?.isFile()) {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, { 'content-type': 'application/octet-stream' });
  fs.createReadStream(file).pipe(response);
});
server.listen(0, '127.0.0.1', () => console.log(server.address().port));
