import { createContext, useContext } from "react";
import type { HelloResponse } from "../types";
import type { Transport } from "./transport";

/**
 * The server the page works with. `local` is this machine's server (the app only), `remote` a
 * paired server reached through the app shell, `origin` the server that served the page (a browser).
 */
export interface ActiveServer {
  kind: "local" | "remote" | "origin";
  /** What the user sees: "This computer", a paired server's saved name, or the page's host. */
  name: string;
  transport: Transport;
  hello: HelloResponse;
}

export interface ServerContextValue {
  server: ActiveServer;
  /** Opens the server picker (app only). */
  switchServer: () => void;
}

export const ServerContext = createContext<ServerContextValue | null>(null);

export function useServer(): ServerContextValue {
  const value = useContext(ServerContext);
  if (!value) throw new Error("useServer is used outside ServerProvider.");
  return value;
}
