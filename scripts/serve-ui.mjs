// Tiny static server to preview the Sotto frontend with bundled fonts over HTTP
// (browsers block @font-face over file://). Run: node scripts/serve-ui.mjs
// Then open http://localhost:5173/settings.html  or  /preview.html
import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const root = path.join(import.meta.dirname, "..", "ui");
const types = {
  ".html": "text/html", ".js": "text/javascript", ".css": "text/css",
  ".woff2": "font/woff2", ".png": "image/png", ".ico": "image/x-icon",
};
const port = 5173;

http.createServer((req, res) => {
  let p = decodeURIComponent(req.url.split("?")[0]);
  if (p === "/") p = "/settings.html";
  const file = path.join(root, p);
  if (!file.startsWith(root) || !fs.existsSync(file)) {
    res.writeHead(404);
    return res.end("not found");
  }
  res.writeHead(200, { "content-type": types[path.extname(file)] || "application/octet-stream" });
  fs.createReadStream(file).pipe(res);
}).listen(port, () => {
  console.log(`Sotto UI preview → http://localhost:${port}/settings.html  and  /preview.html`);
});
