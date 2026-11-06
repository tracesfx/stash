import "./theme";
import { invoke } from "@tauri-apps/api/core";

const note = document.getElementById("note") as HTMLInputElement;
note.focus();

// Native <input>: Ctrl+Backspace and the other standard editing shortcuts work.
note.addEventListener("keydown", async (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    await invoke("note_save", { text: note.value });
  } else if (e.key === "Escape") {
    await invoke("note_close");
  }
});
window.addEventListener("focus", () => note.focus());
