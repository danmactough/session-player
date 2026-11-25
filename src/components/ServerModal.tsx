import { useEffect, useState } from "react";
import * as native from "../lib/native";
import { FetchTransport } from "../lib/transport";
import type { LocalServerState, ServerRef } from "../types";

export type ServerChoice = { kind: "local" } | { kind: "remote"; name: string } | { kind: "origin" };

/**
 * Picks the server the app works with: this computer's or a paired one, which can be added here
 * with a pairing string from `sp pair` on that machine. Shown from Settings to switch, and with
 * no way to dismiss it when the app can't connect at launch.
 */
export function ServerModal({
  local,
  current,
  initialError,
  onChoose,
  onCancel,
}: {
  /** Null while it is being asked for. */
  local: LocalServerState | null;
  current: ServerChoice | null;
  initialError: string | null;
  /** Connects; rejects with why it couldn't. */
  onChoose: (choice: ServerChoice) => Promise<void>;
  onCancel?: () => void;
}) {
  const [paired, setPaired] = useState<ServerRef[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(initialError);
  const [pairing, setPairing] = useState("");
  const [name, setName] = useState("");
  // What installing or starting the service answered; until then, what the parent found.
  const [started, setLocalState] = useState<LocalServerState | null>(null);
  const localState = started ?? local;

  const reloadPaired = () => native.pairedServersList().then(setPaired, (e) => setErr(String(e)));
  useEffect(() => void reloadPaired(), []);

  useEffect(() => {
    if (!onCancel) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !busy && onCancel();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel, busy]);

  const run = async (label: string, work: () => Promise<void>) => {
    setErr(null);
    setBusy(label);
    try {
      await work();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(null);
    }
  };

  const connectLocal = () => run("local", () => onChoose({ kind: "local" }));

  const keepRunning = () =>
    run("keep", async () => {
      setLocalState(await native.localServerKeepRunning());
      await onChoose({ kind: "local" });
    });

  const addPaired = () =>
    run("pair", async () => {
      const added = await native.pairedServerConnect(pairing.trim(), name.trim() || null);
      setPairing("");
      setName("");
      await reloadPaired();
      await onChoose({ kind: "remote", name: added.name });
    });

  const isCurrent = (c: ServerChoice) =>
    current?.kind === c.kind && (c.kind !== "remote" || (current.kind === "remote" && current.name === c.name));

  // A server this app didn't start keeps running without it.
  const canKeepRunning = localState !== null && !(localState.state === "running" && !localState.sidecar);

  return (
    <div className="sheet-backdrop" onClick={() => !busy && onCancel?.()}>
      <div className="sheet server-modal" role="dialog" aria-label="Choose a server" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-head">
          <h2>{current ? "Switch server" : "Choose a server"}</h2>
          {onCancel && (
            <button className="ghost" disabled={!!busy} onClick={onCancel}>
              Cancel
            </button>
          )}
        </div>
        <p className="sheet-note">
          Session Player reads your sessions through a server: one on this computer, or one on another computer you
          have paired with.
        </p>

        <h3 className="settings-head">Local</h3>
        <p className="sheet-note">
          {localState === null && "Checking…"}
          {localState?.state === "running" &&
            (localState.sidecar
              ? `The app is running the server on port ${localState.port}. It stops when you quit Session Player.`
              : `The server is running on port ${localState.port}.`)}
          {localState && localState.state !== "running" && "The app runs the server, which stops when you quit Session Player."}
          {canKeepRunning &&
            " To keep it running in the background, run it as a login item for your user (no administrator access)."}
        </p>
        <div className="sheet-foot">
          {canKeepRunning && (
            <button className="ghost" disabled={!!busy} onClick={() => void keepRunning()}>
              {busy === "keep" ? "Starting…" : "Keep running in the background"}
            </button>
          )}
          <button className="primary" disabled={!!busy || isCurrent({ kind: "local" })} onClick={() => void connectLocal()}>
            {busy === "local" ? "Connecting…" : isCurrent({ kind: "local" }) ? "In use" : "Use local"}
          </button>
        </div>

        <h3 className="settings-head">Paired servers</h3>
        <ul className="roots">
          {paired.map((s) => (
            <li key={s.name}>
              <div>
                <div className="root-path">{s.name}</div>
                <div className="root-meta">
                  {s.address}:{s.port}
                  {s.pinned ? "" : " · plain HTTP"}
                </div>
              </div>
              <button
                className="ghost"
                disabled={!!busy || isCurrent({ kind: "remote", name: s.name })}
                onClick={() => void run(s.name, () => onChoose({ kind: "remote", name: s.name }))}
              >
                {busy === s.name ? "Connecting…" : isCurrent({ kind: "remote", name: s.name }) ? "In use" : "Use"}
              </button>
              <button
                className="ghost danger"
                disabled={!!busy || isCurrent({ kind: "remote", name: s.name })}
                onClick={() => void run(`remove ${s.name}`, () => native.pairedServerRemove(s.name).then(reloadPaired))}
              >
                Remove
              </button>
            </li>
          ))}
          {paired.length === 0 && <li className="none">None yet.</li>}
        </ul>

        <h3 className="settings-head">Pair with another computer</h3>
        <p className="sheet-note">
          Run <code>sp pair</code> on the computer whose sessions you want, and paste the string it prints.
        </p>
        <label className="setting-field">
          <span>Pairing string</span>
          <input value={pairing} onChange={(e) => setPairing(e.target.value)} placeholder="sp1.…" spellCheck={false} />
        </label>
        <label className="setting-field">
          <span>Name</span>
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Its address" />
          <span className="setting-note">What this app calls that server. Leave it empty to use its address.</span>
        </label>
        <div className="sheet-foot">
          <button className="primary" disabled={!!busy || !pairing.trim().startsWith("sp1.")} onClick={() => void addPaired()}>
            {busy === "pair" ? "Pairing…" : "Pair and connect"}
          </button>
        </div>

        {err && <div className="sheet-err">{err}</div>}
      </div>
    </div>
  );
}

/** The secret in a pairing string (`sp1.` + base64url JSON). */
function pairingSecret(text: string): string {
  const encoded = text.trim().replace(/^sp1\./, "");
  if (encoded === text.trim()) throw new Error("A pairing string starts with sp1.");
  const base64 = encoded.replace(/-/g, "+").replace(/_/g, "/");
  let fields: { secret?: string };
  try {
    fields = JSON.parse(atob(base64 + "=".repeat((4 - (base64.length % 4)) % 4)));
  } catch {
    throw new Error("The pairing string is damaged.");
  }
  if (!fields.secret) throw new Error("The pairing string is damaged.");
  return fields.secret;
}

/**
 * Pairs this browser with the server that served the page. The server answers with a cookie the
 * page can't read, so the browser stays paired until the device is removed with `sp unpair`.
 */
export function BrowserPairing({ error, onPaired }: { error: string | null; onPaired: () => Promise<void> }) {
  const [pairing, setPairing] = useState("");
  const [name, setName] = useState("Browser");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(error);

  const pair = async () => {
    setErr(null);
    setBusy(true);
    try {
      const secret = pairingSecret(pairing);
      await new FetchTransport({ baseUrl: "", token: null }).call("connect", { secret, name: name.trim() || "Browser", browser: true });
      await onPaired();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="sheet-backdrop">
      <div className="sheet server-modal" role="dialog" aria-label="Pair this browser">
        <div className="sheet-head">
          <h2>Pair this browser</h2>
        </div>
        <p className="sheet-note">
          Run <code>sp pair</code> on the computer serving this page, and paste the string it prints. The string works once
          and expires after five minutes.
        </p>
        <label className="setting-field">
          <span>Pairing string</span>
          <input value={pairing} onChange={(e) => setPairing(e.target.value)} placeholder="sp1.…" spellCheck={false} autoFocus />
        </label>
        <label className="setting-field">
          <span>Name</span>
          <input value={name} onChange={(e) => setName(e.target.value)} />
          <span className="setting-note">
            How <code>sp devices</code> lists this browser.
          </span>
        </label>
        {err && <div className="sheet-err">{err}</div>}
        <div className="sheet-foot">
          <button className="primary" disabled={busy || !pairing.trim()} onClick={() => void pair()}>
            {busy ? "Pairing…" : "Pair"}
          </button>
        </div>
      </div>
    </div>
  );
}
