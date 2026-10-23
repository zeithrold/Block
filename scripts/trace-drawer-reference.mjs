#!/usr/bin/env node

import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createRequire } from "node:module";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const nativeRoot = path.resolve(scriptDir, "..");
const defaults = {
  prototypeRoot: path.resolve(nativeRoot, "../../Frontend/Block-UI"),
  outDir: path.join(nativeRoot, "target/drawer-motion"),
  port: 5185,
};

function parseArgs(argv) {
  const options = { ...defaults };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (["--prototype-root", "--out-dir", "--port"].includes(arg)) {
      const value = argv[++index];
      if (!value || value.startsWith("--")) throw new Error(`${arg} requires a value`);
      if (arg === "--prototype-root") options.prototypeRoot = path.resolve(value);
      else if (arg === "--out-dir") options.outDir = path.resolve(value);
      else options.port = Number(value);
    }
    else if (arg === "--help" || arg === "-h") {
      process.stdout.write("Usage: node scripts/trace-drawer-reference.mjs [--prototype-root PATH] [--out-dir PATH] [--port PORT]\n");
      process.exit(0);
    } else throw new Error(`Unknown argument: ${arg}`);
  }
  if (!Number.isInteger(options.port) || options.port < 1 || options.port > 65535) {
    throw new Error("--port must be an integer between 1 and 65535");
  }
  return options;
}

async function readPackageVersion(requireFromPrototype, name) {
  try {
    const packagePath = requireFromPrototype.resolve(`${name}/package.json`);
    const packageJson = JSON.parse(await readFile(packagePath, "utf8"));
    return packageJson.version ?? "unknown";
  } catch {
    try {
      let directory = path.dirname(requireFromPrototype.resolve(name));
      while (directory !== path.dirname(directory)) {
        try {
          const pkg = JSON.parse(await readFile(path.join(directory, "package.json"), "utf8"));
          if (pkg.name === name) return pkg.version ?? "unknown";
        } catch {}
        directory = path.dirname(directory);
      }
    } catch {}
    return "unavailable";
  }
}

function harnessSource() {
  return `
import React from "react";
import { createRoot } from "react-dom/client";
import { Check, Search } from "lucide-react";
import {
  Drawer, DrawerClose, DrawerContent, DrawerDescription, DrawerHeader,
  DrawerTitle, DrawerTrigger,
} from "@/components/ui/drawer";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import "@/app/app.css";
import "./harness.css";

const profileRows = [
  { id: "profile-primary", name: "Main Profile", instance: "Minecraft 1.21.4 · Fabric" },
  { id: "profile-vanilla", name: "Vanilla Survival", instance: "Minecraft 1.21.4 · Vanilla" },
  { id: "profile-creative", name: "Creative Build", instance: "Minecraft 1.20.6 · Quilt" },
];
const accountRows = [
  { id: "account-primary", name: "Alex Example", kind: "Microsoft account" },
  { id: "account-secondary", name: "Builder Example", kind: "Microsoft account" },
];

function Harness() {
  const matchedHeight = Number(new URLSearchParams(location.search).get("height")) || null;
  const [open, setOpen] = React.useState(false);
  const [kind, setKind] = React.useState("profile");
  const [search, setSearch] = React.useState("");
  const [status, setStatus] = React.useState("Ready");
  const triggerRef = React.useRef(null);
  const rows = kind === "profile" ? profileRows : accountRows;
  const filteredRows = rows.filter((row) => row.name.toLowerCase().includes(search.trim().toLowerCase()));

  const beginCapture = (nextKind) => {
    setKind(nextKind);
    setSearch("");
    setStatus("Capturing entrance…");
    const startedAt = performance.now();
    const frames = [];
    const transitions = [];
    let ended = false;
    const elapsed = () => performance.now() - startedAt;
    const metrics = (node) => {
      if (!node) return null;
      const rect = node.getBoundingClientRect();
      const style = getComputedStyle(node);
      return {
        x: rect.x, y: rect.y, width: rect.width, height: rect.height,
        opacity: Number(style.opacity), transform: style.transform,
        filter: style.filter, transitionProperty: style.transitionProperty,
        transitionDuration: style.transitionDuration,
        transitionTimingFunction: style.transitionTimingFunction,
        animations: node.getAnimations().map((animation) => {
          const timing = animation.effect?.getComputedTiming();
          return { property: animation.transitionProperty ?? null, currentTime: animation.currentTime,
            startTime: animation.startTime, progress: timing?.progress, duration: timing?.duration };
        }),
      };
    };
    const onTransition = (event) => {
      if (!(event.target instanceof Element)) return;
      const popup = event.target.closest('[data-slot="drawer-popup"]');
      const backdrop = event.target.closest('[data-slot="drawer-overlay"]');
      if (!popup && !backdrop) return;
      transitions.push({
        type: event.type, elapsedMs: elapsed(), performanceNow: performance.now(),
        elapsedTime: event.elapsedTime, propertyName: event.propertyName,
        target: popup ? "popup" : "backdrop",
      });
    };
    const transitionTypes = ["transitionrun", "transitionstart", "transitionend", "transitioncancel"];
    for (const type of transitionTypes) document.addEventListener(type, onTransition, true);

    const tick = (rafTimestamp) => {
      if (ended) return;
      const popup = document.querySelector('[data-slot="drawer-popup"]');
      const overlay = document.querySelector('[data-slot="drawer-overlay"]');
      const children = popup ? [...popup.querySelectorAll('[data-slot="drawer-content"], [data-slot="drawer-header"], [data-slot="input"], [data-slot="scroll-area"], [data-reference-row]')].map((node) => ({
        slot: node.getAttribute("data-slot") ?? "reference-row",
        label: node.getAttribute("data-reference-row"),
        ...metrics(node),
      })) : [];
      frames.push({
        elapsedMs: elapsed(), performanceNow: performance.now(), rafTimestamp,
        viewport: { width: innerWidth, height: innerHeight, dpr: devicePixelRatio },
        reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches,
        visibility: document.visibilityState, focused: document.hasFocus(),
        drawerOpen: Boolean(popup), popup: metrics(popup), backdrop: metrics(overlay), children,
      });
      if (elapsed() >= 800) {
        ended = true;
        for (const type of transitionTypes) document.removeEventListener(type, onTransition, true);
        const captures = JSON.parse(sessionStorage.getItem("drawer-captures") ?? "[]");
        captures.push({
          kind,
          matchedHeight,
          triggerElapsedMs: 0,
          startedAtEpochMs: performance.timeOrigin + startedAt,
          durationMs: elapsed(),
          viewport: { width: innerWidth, height: innerHeight, dpr: devicePixelRatio },
          reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches,
          visibility: document.visibilityState, focused: document.hasFocus(),
          frames, transitions,
        });
        sessionStorage.setItem("drawer-captures", JSON.stringify(captures));
        fetch("/__result", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ captures }) })
          .then((response) => {
            if (!response.ok) throw new Error("Local result save failed");
            setStatus("Done — saved drawer-prototype.json");
          })
          .catch((error) => setStatus(error.message || "Could not save capture"));
        return;
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  };

  return <>
    <Drawer open={open} onOpenChange={(next) => { setOpen(next); if (next) beginCapture(kind); }} showSwipeHandle swipeDirection="down">
    <main className="trace-controls">
      <strong>Base UI drawer motion reference</strong>
      <p>Choose the Home selector content, then open the drawer to capture its entrance.</p>
      <div className="trace-actions">
        <button type="button" onClick={() => { setKind("profile"); setSearch(""); }}>Profile content</button>
        <button type="button" onClick={() => { setKind("account"); setSearch(""); }}>Account content</button>
        <DrawerTrigger render={<button ref={triggerRef} type="button">Open drawer</button>} />
        <DrawerClose render={<button type="button" disabled={!open}>Close drawer</button>} />
      </div>
      <span role="status">{status}</span>
    </main>
      <DrawerContent
        finalFocus={triggerRef}
        style={{ maxHeight: "80%", ...(matchedHeight ? {height: matchedHeight} : {}) }}
        className="motion-reduce:transition-none"
      >
        <DrawerHeader className="gap-2 px-6 pt-5 text-left">
          <DrawerTitle>{kind === "profile" ? "Switch Profile" : "Switch Account"}</DrawerTitle>
          <DrawerDescription>{kind === "profile" ? "Choose how you want to play." : "Choose an account to play with."}</DrawerDescription>
        </DrawerHeader>
        <div className="relative mx-6 mt-4 shrink-0">
          <Search className="pointer-events-none absolute top-1/2 left-4 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input
            aria-label={kind === "profile" ? "Search profiles" : "Search accounts"}
            placeholder={kind === "profile" ? "Search profiles..." : "Search accounts..."}
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            className="h-10 border-0 bg-muted/60 py-2 pr-4 pl-11 focus-visible:bg-muted focus-visible:ring-0 active:bg-muted dark:bg-muted/40 dark:focus-visible:bg-muted dark:active:bg-muted"
          />
        </div>
        <ScrollArea className="mt-4 h-80 min-h-0" aria-label={kind === "profile" ? "Profile list" : "Account list"}>
          <div role="radiogroup" aria-label={kind === "profile" ? "Profiles" : "Accounts"} className="grid-cols-1 gap-0">
            {filteredRows.map((row, index) => <label key={row.id} data-reference-row={row.id} className="flex min-h-20 w-full cursor-pointer items-center gap-3 px-6 py-4 transition-colors hover:bg-accent/50 has-[:focus-visible]:bg-accent/50 has-[[data-checked]]:bg-accent">
              <span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-muted text-sm font-medium">{row.name.slice(0, 1)}</span>
              <span className="flex min-w-0 flex-1 flex-col gap-1">
                <span className="truncate text-sm font-medium">{row.name}</span>
                <span className="text-xs text-muted-foreground">{row.instance ?? row.kind}</span>
              </span>
              <span className="flex size-5 shrink-0 items-center justify-center" aria-hidden="true">{index === 0 && <Check className="size-5" />}</span>
            </label>)}
            {filteredRows.length === 0 && <p role="status" className="px-6 py-8 text-center text-sm text-muted-foreground">No {kind === "profile" ? "profiles" : "accounts"} found.</p>}
          </div>
        </ScrollArea>
      </DrawerContent>
    </Drawer>
  </>;
}

createRoot(document.getElementById("root")).render(<Harness />);
`;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const prototypeRoot = path.resolve(options.prototypeRoot);
  const outDir = path.resolve(options.outDir);
  const harnessDir = path.join(outDir, "reference-harness");
  const requireFromPrototype = createRequire(path.join(prototypeRoot, "package.json"));
  const protoPackage = JSON.parse(await readFile(path.join(prototypeRoot, "package.json"), "utf8"));
  const componentPath = path.join(prototypeRoot, "components/ui/drawer.tsx");
  const cssPath = path.join(prototypeRoot, "app/app.css");
  await readFile(componentPath);
  await readFile(cssPath);
  await mkdir(harnessDir, { recursive: true });

  const vitePath = requireFromPrototype.resolve("vite");
  const tailwindPath = requireFromPrototype.resolve("@tailwindcss/vite");
  const { createServer } = await import(pathToFileURL(vitePath));
  const tailwindModule = await import(pathToFileURL(tailwindPath));
  const packageVersions = {};
  for (const name of ["vite", "@tailwindcss/vite", "react", "react-dom", "@base-ui/react", "sonner", "next-themes", "lucide-react"]) {
    packageVersions[name] = await readPackageVersion(requireFromPrototype, name);
  }

  const html = `<!doctype html><html lang="en"><head><meta charset="UTF-8"/><meta name="viewport" content="width=device-width, initial-scale=1.0"/><title>Drawer motion reference</title></head><body><div id="root"></div><script type="module" src="/@fs${path.join(harnessDir, "harness.jsx")}"></script></body></html>`;
  const harnessCss = `@source ${JSON.stringify(prototypeRoot.replaceAll("\\", "/"))};\n.trace-controls { position: fixed; z-index: 2147483000; top: 16px; left: 16px; display: grid; gap: 8px; max-width: 420px; padding: 14px; border: 1px solid #cbd5e1; border-radius: 8px; background: rgba(255,255,255,.96); color: #111827; font: 14px/1.4 system-ui, sans-serif; box-shadow: 0 4px 18px rgba(0,0,0,.12); }\n.trace-controls p { margin: 0; }\n.trace-actions { display: flex; flex-wrap: wrap; gap: 6px; }\n.trace-actions button { width: fit-content; padding: 7px 12px; border: 0; border-radius: 6px; background: #111827; color: white; font: inherit; cursor: pointer; }\n.trace-actions button:disabled { opacity: .5; cursor: default; }\n`;
  await writeFile(path.join(harnessDir, "index.html"), html);
  await writeFile(path.join(harnessDir, "harness.jsx"), harnessSource());
  await writeFile(path.join(harnessDir, "harness.css"), harnessCss);

  const captureMetadata = {
    generatedAt: new Date().toISOString(),
    prototypeRoot,
    componentPath,
    cssPath,
    packageName: protoPackage.name ?? null,
    packageVersions,
    viteConfig: {
      configFile: false,
      root: prototypeRoot,
      alias: { "@": prototypeRoot },
      fsAllow: [prototypeRoot, outDir],
      optimizeDepsInclude: ["react", "react/jsx-runtime", "react/jsx-dev-runtime", "react-dom/client", "sonner", "next-themes", "lucide-react", "@base-ui/react/drawer"],
      dedupe: Object.keys(packageVersions),
      plugins: ["@tailwindcss/vite"],
    },
  };

  const diagnosticPlugin = {
    name: "drawer-trace-local-result",
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        if (request.method === "GET" && request.url?.split("?")[0] === "/") {
          response.setHeader("content-type", "text/html; charset=utf-8");
          response.end(html);
          return;
        }
        next();
      });
      server.middlewares.use("/__result", async (request, response) => {
        if (request.method !== "POST") {
          response.statusCode = 405;
          response.end("POST only");
          return;
        }
        try {
          const chunks = [];
          let size = 0;
          for await (const chunk of request) {
            size += chunk.length;
            if (size > 100 * 1024 * 1024) throw new Error("Capture exceeds 100 MiB");
            chunks.push(chunk);
          }
          const payload = JSON.parse(Buffer.concat(chunks).toString("utf8"));
          const destination = path.join(outDir, "drawer-prototype.json");
          await writeFile(destination, `${JSON.stringify({ ...captureMetadata, ...payload }, null, 2)}\n`);
          response.setHeader("content-type", "application/json");
          response.end(JSON.stringify({ saved: destination }));
          process.stdout.write(`Saved drawer capture: ${destination} (${payload.captures?.length ?? 0} captures)\n`);
        } catch (error) {
          response.statusCode = 400;
          response.end(error instanceof Error ? error.message : String(error));
        }
      });
    },
  };

  const optimizeDepsInclude = ["react", "react/jsx-runtime", "react/jsx-dev-runtime", "react-dom/client", "sonner", "next-themes", "lucide-react", "@base-ui/react/drawer"];
  const server = await createServer({
    configFile: false,
    root: prototypeRoot,
    plugins: [tailwindModule.default(), diagnosticPlugin],
    resolve: {
      alias: { "@": prototypeRoot },
      dedupe: Object.keys(packageVersions),
    },
    optimizeDeps: { include: optimizeDepsInclude },
    server: {
      host: "127.0.0.1",
      port: options.port,
      strictPort: true,
      fs: { allow: [prototypeRoot, outDir] },
    },
    appType: "custom",
    logLevel: "info",
  });

  const close = async () => {
    await server.close();
    process.exit(0);
  };
  process.once("SIGINT", close);
  process.once("SIGTERM", close);
  await server.listen();
  process.stdout.write(`Drawer reference harness: http://127.0.0.1:${options.port}/\n`);
  process.stdout.write(`Output directory: ${outDir}\n`);
  process.stdout.write("Open the page; select Profile content or Account content, then click Open drawer. Each opening captures 800 ms and saves drawer-prototype.json.\n");
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
  process.exitCode = 1;
});
