// A stand-in for the GitHub API and the health check that serves recorded responses, for
// script/staging-status-test. A GET of <path> answers with <dir><path>.json, the query ignored, and
// with the HTTP status in <dir><path>.status if that file exists, 200 otherwise; a missing file is a
// 404. It reads the files on each request, so a test can change them between runs. A request with
// an Authorization header fails with 400: the command reads only public data.
// Usage: node recorded-api-stub.mjs <dir> <port file>; it listens on 127.0.0.1 and writes its port
// to <port file> once it accepts connections.
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';

const [dir, portFile] = process.argv.slice(2);

function reply(req) {
  if (req.headers.authorization !== undefined) return [400, '{"message":"Authorization sent"}'];
  if (req.method !== 'GET') return [405, '{"message":"Method Not Allowed"}'];
  const file = path.join(dir, decodeURIComponent(new URL(req.url, 'http://stub').pathname));
  if (!file.startsWith(path.join(dir, path.sep)) || !fs.existsSync(`${file}.json`)) {
    return [404, '{"message":"Not Found"}'];
  }
  const status = fs.existsSync(`${file}.status`)
    ? Number(fs.readFileSync(`${file}.status`, 'utf8'))
    : 200;
  return [status, fs.readFileSync(`${file}.json`)];
}

const server = http.createServer((req, res) => {
  req.resume();
  req.on('end', () => {
    const [status, body] = reply(req);
    res.writeHead(status, { 'content-type': 'application/json' });
    res.end(body);
  });
});
server.listen(0, '127.0.0.1', () => fs.writeFileSync(portFile, String(server.address().port)));
