// Renders icons-src/app.svg -> app icon source (1024) and icons-src/{normal,paused,warn}.svg -> 64px tray icons. Then run: npx tauri icon icons-src/app-1024.png
import { Resvg } from "@resvg/resvg-js";
import { readFileSync, writeFileSync } from "node:fs";

const render = (svg, size) => new Resvg(readFileSync(svg), { fitTo: { mode: "width", value: size } }).render().asPng();

writeFileSync("icons-src/app-1024.png", render("icons-src/app.svg", 1024));
writeFileSync("public/logo.png", render("icons-src/app.svg", 128)); // gallery header: the same mark as the taskbar
for (const [name, svg] of [["normal", "normal"], ["paused", "paused"], ["warn", "warn"]]) {
  writeFileSync(`src-tauri/icons/tray-${name}.png`, render(`icons-src/${svg}.svg`, 64));
}
console.log("icons rendered");
