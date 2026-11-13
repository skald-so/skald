// Serves the files of <dir> over HTTP on 127.0.0.1, as GitHub serves release files: each as
// application/octet-stream, 404 for any other path; over HTTPS with the certificate <cert> and its
// key <key>, PEM files. Prints the port it listens on, a free one, then the path of each request,
// and runs until killed. For the tests of the installers and the npm package:
// script/installers-test, script/npm-package-test.
// Usage: node script/lib/serve.mjs <dir> [<cert> <key>]
import fs from 'node:fs';
import http from 'node:http';
import https from 'node:https';
import path from 'node:path';

const [dir, cert, key] = [path.resolve(process.argv[2]), process.argv[3], process.argv[4]];
const handle = (request, response) => {
  console.log(request.url);
  const name = decodeURIComponent(new URL(request.url, 'http://127.0.0.1').pathname.slice(1));
  const file = path.join(dir, name);
  if (request.method !== 'GET' || name.includes('/') || name.includes('\\') || !fs.statSync(file, { throwIfNoEntry: false })?.isFile()) {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, { 'content-type': 'application/octet-stream' });
  fs.createReadStream(file).pipe(response);
};
const server = cert ? https.createServer({ cert: fs.readFileSync(cert), key: fs.readFileSync(key) }, handle) : http.createServer(handle);
server.listen(0, '127.0.0.1', () => console.log(server.address().port));
