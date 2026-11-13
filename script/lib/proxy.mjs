// A stand-in HTTPS proxy on 127.0.0.1: it tunnels each CONNECT, whatever host it names, to
// 127.0.0.1:<port>, and answers any other request with 502. Prints the port it listens on, a free
// one, then each request as `<method> <target>`, and runs until killed. For script/npm-package-test.
// Usage: node script/lib/proxy.mjs <port>
import http from 'node:http';
import net from 'node:net';

const upstream = Number(process.argv[2]);
const proxy = http.createServer((request, response) => {
  console.log(`${request.method} ${request.url}`);
  response.writeHead(502).end();
});
proxy.on('connect', (request, client, head) => {
  console.log(`CONNECT ${request.url}`);
  const server = net.connect(upstream, '127.0.0.1', () => {
    client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
    server.write(head);
    server.pipe(client);
    client.pipe(server);
  });
  const close = () => {
    client.destroy();
    server.destroy();
  };
  server.on('error', close);
  client.on('error', close);
});
proxy.listen(0, '127.0.0.1', () => console.log(proxy.address().port));
