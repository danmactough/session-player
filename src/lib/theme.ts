import { getCurrentWindow } from "@tauri-apps/api/window";

export type Theme = "system" | "light" | "dark";

const KEY = "theme";

/**
 * Applies the theme to the page (data-theme on <html>, which the CSS palette
 * keys off) and to the native window, so the title bar and traffic lights match.
 * The choice is also kept in localStorage so the first paint after launch
 * uses it, before the saved preferences arrive from the backend.
 */
export function applyTheme(theme: Theme) {
  const root = document.documentElement;
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // Storage can be unavailable; the saved preference still applies once loaded.
  }
  try {
    void getCurrentWindow()
      .setTheme(theme === "system" ? null : theme)
      .catch(() => {});
  } catch {
    // Not running inside Tauri (browser preview).
  }
}

export function storedTheme(): Theme {
  try {
    const t = localStorage.getItem(KEY);
    if (t === "light" || t === "dark") return t;
  } catch {
    // Fall through to the default.
  }
  return "system";
}
