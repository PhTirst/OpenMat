// Installed before page scripts so reloading cannot discard the desktop session,
// even while the workbench is loading. F5 execution is handled by the workbench.
window.addEventListener("keydown", (event) => {
  const refreshKey = event.key === "F5" && !event.altKey && !event.metaKey;
  const refreshChord = (event.ctrlKey || event.metaKey) &&
    !event.altKey && event.key.toLowerCase() === "r";
  if (refreshKey || refreshChord) {
    event.preventDefault();
  }
}, { capture: true });
