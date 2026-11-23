import { Fragment, useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { dispatchServerEvent, dispatchStreamOpen, setTransport } from "../api";
import { BrowserPairing, ServerModal, type ServerChoice } from "../components/ServerModal";
import type { HelloResponse, LocalServerState } from "../types";
import { setHome } from "./format";
import * as native from "./native";
import { FetchTransport, RelayTransport, ServerError, type Transport } from "./transport";
import { ServerContext, type ActiveServer } from "./useServer";

const API_VERSION = 1;
const CHOICE_KEY = "server";
/** How long the first paint waits for the event stream, so a job started at once isn't missed. */
const STREAM_WAIT_MS = 3000;

function storedChoice(): string | null {
  try {
    return localStorage.getItem(CHOICE_KEY);
  } catch {
    return null;
  }
}

function storeChoice(choice: ServerChoice) {
  try {
    localStorage.setItem(CHOICE_KEY, choice.kind === "remote" ? `remote:${choice.name}` : choice.kind);
  } catch {
    // The choice just isn't remembered; local stays the default.
  }
}

async function open(choice: ServerChoice): Promise<ActiveServer> {
  let transport: Transport;
  let name: string;
  if (choice.kind === "local") {
    // `npm run app` loads the page from Vite's dev server, an origin the server refuses, so in
    // development calls go through Vite's /api proxy, which presents the server's own origin.
    const base = (url: string) => (import.meta.env.DEV ? "" : url);
    const c = await native.localServerConnection();
    const refresh = () => native.localServerConnection().then((r) => ({ baseUrl: base(r.url), token: r.token }));
    transport = new FetchTransport({ baseUrl: base(c.url), token: c.token, refresh });
    name = "This computer";
  } else if (choice.kind === "remote") {
    transport = new RelayTransport({ server: choice.name, relay: native.relay });
    name = choice.name;
  } else {
    transport = new FetchTransport({ baseUrl: "", token: null });
    name = window.location.host;
  }
  const hello = await transport.call<HelloResponse>("hello", {});
  if (hello.api_version !== API_VERSION) {
    throw new Error(`${name} speaks API version ${hello.api_version}; this app speaks ${API_VERSION}. Update the older one.`);
  }
  return { kind: choice.kind, name, transport, hello };
}

type Phase =
  | { phase: "starting" }
  | { phase: "choosing"; local: LocalServerState | null; error: string | null }
  | { phase: "pairing-browser"; error: string | null }
  | { phase: "failed"; error: string };

/**
 * Connects the page to one server before the app renders. In the app that is the local server
 * when it runs (or the server chosen last time); otherwise the first-run picker. In a browser it
 * is the page's own origin, after pairing the browser if it isn't yet.
 */
export function ServerProvider({ children }: { children: ReactNode }) {
  const [active, setActive] = useState<ActiveServer | null>(null);
  const [phase, setPhase] = useState<Phase>({ phase: "starting" });
  // The picker over a connected app, opened from Settings.
  const [switching, setSwitching] = useState<{ local: LocalServerState | null; error: string | null } | null>(null);
  // Bumped on every switch, so the app remounts with nothing from the previous server.
  const [generation, setGeneration] = useState(0);
  const closeStream = useRef<(() => void) | null>(null);

  const activate = useCallback(async (server: ActiveServer) => {
    closeStream.current?.();
    setTransport(server.transport);
    setHome(server.hello.home);
    let opened = false;
    await new Promise<void>((resolve) => {
      const timer = setTimeout(resolve, STREAM_WAIT_MS);
      closeStream.current = server.transport.events({
        lastId: null,
        onEvent: dispatchServerEvent,
        onOpen: () => {
          if (opened) dispatchStreamOpen();
          opened = true;
          clearTimeout(timer);
          resolve();
        },
      });
    });
    setActive(server);
    setGeneration((g) => g + 1);
  }, []);

  const choose = useCallback(
    async (choice: ServerChoice) => {
      const server = await open(choice);
      storeChoice(choice);
      await activate(server);
      setSwitching(null);
    },
    [activate],
  );

  useEffect(() => {
    let live = true;
    void (async () => {
      if (!native.isApp) {
        try {
          const server = await open({ kind: "origin" });
          if (live) await activate(server);
        } catch (e) {
          if (!live) return;
          if (e instanceof ServerError && e.code === "unauthenticated") setPhase({ phase: "pairing-browser", error: null });
          else setPhase({ phase: "failed", error: String(e) });
        }
        return;
      }
      const saved = storedChoice();
      if (saved?.startsWith("remote:")) {
        try {
          const server = await open({ kind: "remote", name: saved.slice("remote:".length) });
          if (live) await activate(server);
          return;
        } catch {
          // Unreachable or no longer paired; fall back to the local server.
        }
      }
      let local: LocalServerState | null = null;
      let error: string | null = null;
      try {
        local = await native.localServerState();
        if (local.state === "running") {
          const server = await open({ kind: "local" });
          if (live) await activate(server);
          return;
        }
      } catch (e) {
        error = String(e);
      }
      if (live) setPhase({ phase: "choosing", local, error });
    })();
    return () => {
      live = false;
    };
  }, [activate]);

  useEffect(() => () => closeStream.current?.(), []);

  const switchServer = useCallback(() => {
    if (!native.isApp) return;
    setSwitching({ local: null, error: null });
    native.localServerState().then(
      (local) => setSwitching((s) => s && { ...s, local }),
      (e) => setSwitching((s) => s && { ...s, error: String(e) }),
    );
  }, []);

  if (active) {
    return (
      <ServerContext.Provider value={{ server: active, switchServer }}>
        <Fragment key={generation}>{children}</Fragment>
        {switching && (
          <ServerModal
            local={switching.local}
            current={active.kind === "remote" ? { kind: "remote", name: active.name } : { kind: active.kind }}
            initialError={switching.error}
            onChoose={choose}
            onCancel={() => setSwitching(null)}
          />
        )}
      </ServerContext.Provider>
    );
  }
  switch (phase.phase) {
    case "starting":
      return <div className="server-wait pulse">Connecting…</div>;
    case "choosing":
      return <ServerModal local={phase.local} current={null} initialError={phase.error} onChoose={choose} />;
    case "pairing-browser":
      return (
        <BrowserPairing
          key={phase.error ?? ""}
          error={phase.error}
          onPaired={async () => {
            try {
              await activate(await open({ kind: "origin" }));
            } catch (e) {
              setPhase({ phase: "pairing-browser", error: String(e) });
            }
          }}
        />
      );
    case "failed":
      return (
        <div className="server-wait">
          <p>Couldn&rsquo;t reach the server: {phase.error}</p>
          <button className="primary" onClick={() => window.location.reload()}>
            Try again
          </button>
        </div>
      );
  }
}
