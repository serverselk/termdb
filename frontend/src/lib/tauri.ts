import { invoke } from "@tauri-apps/api/core";

// Mirrors termdb-core models (hand-written until tauri-specta lands).
export type Engine = "mysql" | "postgres";

export interface ConnectionConfig {
  id: number | null;
  name: string;
  engine: Engine;
  host: string;
  port: number;
  username: string;
  database?: string | null;
  ssl: boolean;
}

export interface Pong {
  payload: string;
  latency_ms: number;
}

export interface Palette {
  bg: string;
  card: string;
  raised: string;
  hover: string;
  grid: string;
  border_strong: string;
  text: string;
  text_dim: string;
  blue: string;
  blue_dark: string;
  blue_soft: string;
  red: string;
  red_dark: string;
  green: string;
  amber: string;
}

export const ping = (payload: string): Promise<Pong> =>
  invoke("ping", { payload });

export const listConnections = (): Promise<ConnectionConfig[]> =>
  invoke("list_connections");

export const getPalette = (): Promise<Palette> => invoke("get_palette");

/** Apply the Rust-side palette as CSS variables on :root. */
export function applyPalette(p: Palette): void {
  const root = document.documentElement;
  const entries: Array<[string, string]> = [
    ["--bg", p.bg],
    ["--card", p.card],
    ["--raised", p.raised],
    ["--hover", p.hover],
    ["--grid", p.grid],
    ["--border-strong", p.border_strong],
    ["--text", p.text],
    ["--text-dim", p.text_dim],
    ["--blue", p.blue],
    ["--blue-dark", p.blue_dark],
    ["--blue-soft", p.blue_soft],
    ["--red", p.red],
    ["--red-dark", p.red_dark],
    ["--green", p.green],
    ["--amber", p.amber],
  ];
  for (const [key, value] of entries) root.style.setProperty(key, value);
}
