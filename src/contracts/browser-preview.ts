export function isBrowserPreview(): boolean {
  return shouldUseBrowserPreview(import.meta.env.MODE, "__TAURI_INTERNALS__" in window);
}

export function shouldUseBrowserPreview(mode: string, hasTauriInternals: boolean): boolean {
  return (mode === "development" || mode === "browser-preview") && !hasTauriInternals;
}
