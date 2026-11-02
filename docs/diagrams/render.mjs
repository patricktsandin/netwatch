// Renders every Graphviz source in this directory (`*.dot`) to an SVG beside it.
// Run with `npm run diagrams` in web/, which pins the Graphviz build (a WebAssembly
// one, so nothing needs installing on the host). Commit the SVGs with their sources.
import { createRequire } from "node:module";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(join(here, "../../web/package.json"));
const { Graphviz } = await import(require.resolve("@hpcc-js/wasm-graphviz"));
const graphviz = await Graphviz.load();

for (const file of readdirSync(here).filter((f) => f.endsWith(".dot")).sort()) {
  const svg = graphviz.dot(readFileSync(join(here, file), "utf8"), "svg");
  writeFileSync(join(here, file.replace(/\.dot$/, ".svg")), svg);
  console.log(`docs/diagrams/${file} → ${file.replace(/\.dot$/, ".svg")}`);
}
