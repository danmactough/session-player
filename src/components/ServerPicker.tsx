import { useState } from "react";
import { useServer } from "../lib/useServer";
import { Menu } from "./Menu";

type Key = "local" | `remote:${string}`;

/**
 * The nav strip's button for switching servers, shown once a server has been paired. Its tooltip
 * names the server in use.
 */
export function ServerPicker({ onError }: { onError: (message: string) => void }) {
  const { server, paired, connect } = useServer();
  const [connecting, setConnecting] = useState<string | null>(null);
  if (!paired.length) return null;

  const current: Key = server.kind === "remote" ? `remote:${server.name}` : "local";
  const options: { key: Key; label: string }[] = [
    { key: "local", label: "local" },
    ...paired.map((s) => ({ key: `remote:${s.name}` as const, label: s.name })),
  ];

  const choose = async (key: Key) => {
    if (key === current || connecting) return;
    const name = options.find((o) => o.key === key)!.label;
    setConnecting(name);
    try {
      await connect(key === "local" ? { kind: "local" } : { kind: "remote", name: key.slice("remote:".length) });
    } catch (e) {
      onError(`Couldn't connect to ${name}: ${e}`);
    } finally {
      setConnecting(null);
    }
  };

  const title = connecting ? `Connecting to ${connecting}…` : `Server Connection • ${server.name}`;
  return (
    <Menu
      className="server-picker"
      label="Server Connection"
      header="Server Connection"
      title={title}
      value={current}
      options={options}
      onChange={(key) => void choose(key)}
      trigger={
        <svg className={connecting ? "pulse" : ""} width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <path d="m19 5 3-3M2 22l3-3" />
          <path d="M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4Z" />
          <path d="M7.5 13.5 10 11M10.5 16.5 13 14" />
          <path d="m12 6 6 6 2.3-2.3a2.4 2.4 0 0 0 0-3.4l-2.6-2.6a2.4 2.4 0 0 0-3.4 0Z" />
        </svg>
      }
    />
  );
}
