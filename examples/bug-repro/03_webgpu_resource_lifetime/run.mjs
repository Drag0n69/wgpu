// Builds the repro for the web, serves it on 127.0.0.1 and runs it in headless
// Chrome. Prints the JSON the page reports.
//
//   node examples/bug-repro/03_webgpu_resource_lifetime/run.mjs
//
// Environment:
//   CHROME        path of the Chrome or Chromium executable (default: the
//                 usual install locations)
//   WASM_BINDGEN  path of the wasm-bindgen CLI (default: `wasm-bindgen`); its
//                 version must match the one in Cargo.lock
//   REPRO_OUT     file the JSON is also written to (optional)
//   REPRO_FLAGS   extra Chrome flags, separated by spaces (default: the
//                 software adapter, so that no GPU is needed)

import { execFileSync, spawn } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../../..");
const chrome =
  process.env.CHROME ||
  [
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ].find((path) => existsSync(path));
if (!chrome) {
  console.error("set CHROME to the path of the Chrome executable");
  process.exit(2);
}
const wasmBindgen = process.env.WASM_BINDGEN || "wasm-bindgen";
const flags = (
  process.env.REPRO_FLAGS ?? "--enable-unsafe-swiftshader --use-webgpu-adapter=swiftshader"
)
  .split(" ")
  .filter(Boolean);

const run = (program, args) => execFileSync(program, args, { cwd: root, stdio: "inherit" });

run("cargo", [
  "build",
  "--release",
  "--target",
  "wasm32-unknown-unknown",
  "--package",
  "wgpu-bug-repro-03-webgpu-resource-lifetime",
]);
const targetDir = process.env.CARGO_TARGET_DIR || join(root, "target");
const dist = join(targetDir, "bug-repro-03-web");
rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });
run(wasmBindgen, [
  join(
    targetDir,
    "wasm32-unknown-unknown/release/wgpu_bug_repro_03_webgpu_resource_lifetime.wasm",
  ),
  "--out-dir",
  dist,
  "--out-name",
  "repro",
  "--target",
  "web",
]);
cpSync(join(here, "web/index.html"), join(dist, "index.html"));

const types = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm" };
let browser;
let profile;
const finish = (code) => {
  if (browser) {
    try {
      if (process.platform === "win32") {
        execFileSync("taskkill", ["/PID", String(browser.pid), "/T", "/F"], { stdio: "ignore" });
      } else {
        browser.kill("SIGKILL");
      }
    } catch {
      // Already gone.
    }
  }
  if (profile) {
    try {
      rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    } catch {
      // Chrome may still hold a file for a moment.
    }
  }
  process.exit(code);
};

const server = createServer((request, response) => {
  if (request.method === "POST" && request.url === "/result") {
    let body = "";
    request.on("data", (chunk) => (body += chunk));
    request.on("end", () => {
      response.end("ok");
      console.log(body);
      if (process.env.REPRO_OUT) writeFileSync(process.env.REPRO_OUT, body);
      finish(body.includes('"error"') ? 1 : 0);
    });
    return;
  }
  const name = request.url === "/" ? "/index.html" : request.url.split("?")[0];
  try {
    const content = readFileSync(join(dist, name));
    response.writeHead(200, { "Content-Type": types[extname(name)] || "application/octet-stream" });
    response.end(content);
  } catch {
    response.writeHead(404);
    response.end();
  }
});

server.listen(0, "127.0.0.1", () => {
  const url = `http://127.0.0.1:${server.address().port}/`;
  profile = mkdtempSync(join(tmpdir(), "wgpu-repro-"));
  browser = spawn(
    chrome,
    [
      "--headless=new",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-extensions",
      `--user-data-dir=${profile}`,
      "--enable-unsafe-webgpu",
      ...flags,
      url,
    ],
    { stdio: "ignore" },
  );
  setTimeout(() => {
    console.error("timeout: the page did not report a result");
    finish(3);
  }, 300_000);
});
