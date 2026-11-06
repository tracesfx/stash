// Renders icons-src/*.svg -> app icon source (1024) and 64px tray icons. Then run: npx tauri icon icons-src/app-1024.png
import { Resvg } from "@resvg/resvg-js";
import { readFileSync, writeFileSync } from "node:fs";

const render = (svg, size) => new Resvg(readFileSync(svg), { fitTo: { mode: "width", value: size } }).render().asPng();

writeFileSync("icons-src/app-1024.png", render("icons-src/normal.svg", 1024));
for (const [name, svg] of [["normal", "normal"], ["paused", "paused"], ["warn", "warn"]]) {
  writeFileSync(`src-tauri/icons/tray-${name}.png`, render(`icons-src/${svg}.svg`, 64));
}
console.log("icons rendered");
