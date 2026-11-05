import type { UpdateStatus } from "../lib/useUpdater";

interface Props {
  status: UpdateStatus;
  onInstall: () => void;
  onRestart: () => void;
  onLater: () => void;
}

export function UpdateBanner({ status, onInstall, onRestart, onLater }: Props) {
  if (status.phase === "idle") return null;
  return (
    <div className="update-banner" role="status">
      {status.phase === "available" && (
        <>
          <div className="update-text">Version {status.version} is available</div>
          <div className="update-actions">
            <button className="primary" onClick={onInstall}>
              Install and restart
            </button>
            <button className="ghost" onClick={onLater}>
              Later
            </button>
          </div>
        </>
      )}
      {status.phase === "downloading" && (
        <div className="update-text">
          Downloading version {status.version}
          {status.percent === null ? "…" : ` · ${status.percent}%`}
        </div>
      )}
      {status.phase === "ready" && (
        <>
          <div className="update-text">Version {status.version} is ready to install</div>
          <div className="update-actions">
            <button className="primary" onClick={onRestart}>
              Restart now
            </button>
            <button className="ghost" onClick={onLater}>
              Later
            </button>
          </div>
        </>
      )}
      {status.phase === "installing" && <div className="update-text">Installing version {status.version}…</div>}
    </div>
  );
}
