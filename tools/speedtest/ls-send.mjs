// LocalSend protocol v2 SCRIPTED SENDER (PC -> the phone's LocalSend app). Outgoing HTTPS connections only; opens no listener.
// Speaks /api/localsend/v2/prepare-upload and /upload on port 53317. The phone's LocalSend needs Quick Save on and must be in the foreground.
// It measures the phone's receive path, not the LocalSend PC app. Usage: node ls-send.mjs <phone-ip> <file> [<file>...]   (--info: only GET /info)
import https from 'node:https'; import fs from 'node:fs'; import path from 'node:path'; import crypto from 'node:crypto';
const [ip, ...rest] = process.argv.slice(2);
const infoOnly = rest[0] === '--info';
const base = '/api/localsend/v2';
const agent = new https.Agent({ rejectUnauthorized: false, keepAlive: true, cert: fs.readFileSync(process.env.LS_CLIENT_CERT), key: fs.readFileSync(process.env.LS_CLIENT_KEY) });
function req(method, p, body, stream) {
  return new Promise((res, rej) => {
    const headers = {};
    if (body) { headers['content-type'] = 'application/json'; headers['content-length'] = Buffer.byteLength(body); }
    if (stream) { headers['content-type'] = 'application/octet-stream'; headers['content-length'] = stream.size; }
    const r = https.request({ host: ip, port: 53317, path: base + p, method, agent, headers, timeout: 600000 }, (resp) => {
      const ch = []; resp.on('data', (d) => ch.push(d)); resp.on('end', () => res({ status: resp.statusCode, body: Buffer.concat(ch).toString() }));
    });
    r.on('error', rej); r.on('timeout', () => r.destroy(new Error('timeout')));
    if (stream) fs.createReadStream(stream.path, { highWaterMark: 1 << 20 }).pipe(r); else r.end(body);
  });
}
if (infoOnly) { const r = await req('GET', '/info'); console.log(r.status, r.body.replace(/"fingerprint":"[^"]*"/, '"fingerprint":"..."')); process.exit(0); }
const files = rest.map((f) => ({ f, size: fs.statSync(f).size }));
const fmap = {}; files.forEach((x, i) => { const id = 'f' + (i + 1); fmap[id] = { id, fileName: path.basename(x.f), size: x.size, fileType: x.f.endsWith('.jpg') ? 'image/jpeg' : 'application/octet-stream' }; });
const info = { alias: 'speedtest-script', version: '2.1', deviceModel: 'script', deviceType: 'desktop', fingerprint: 'speedtest-' + crypto.randomUUID(), port: 53317, protocol: 'https', download: false };
const t0 = Date.now();
const prep = await req('POST', '/prepare-upload', JSON.stringify({ info, files: fmap }));
if (prep.status !== 200) { console.error('prepare-upload', prep.status, prep.body); process.exit(2); }
const j = JSON.parse(prep.body); const t1 = Date.now();
let total = 0;
for (let i = 0; i < files.length; i++) {
  const id = 'f' + (i + 1);
  const r = await req('POST', `/upload?sessionId=${j.sessionId}&fileId=${id}&token=${j.files[id]}`, null, { path: files[i].f, size: files[i].size });
  if (r.status !== 200) { console.error('upload', id, r.status, r.body); process.exit(3); }
  total += files[i].size;
}
const t2 = Date.now();
console.log(JSON.stringify({ files: files.length, bytes: total, startedUtcMs: t0, prepareMs: t1 - t0, uploadSec: (t2 - t1) / 1000, totalSec: (t2 - t0) / 1000, uploadMBps: +(total / 1e6 / ((t2 - t1) / 1000)).toFixed(1), totalMBps: +(total / 1e6 / ((t2 - t0) / 1000)).toFixed(1) }));


