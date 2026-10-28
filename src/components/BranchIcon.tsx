/** A git-style fork: a trunk with one line splitting off to the right. */
export function BranchIcon({ className }: { className?: string }) {
  return (
    <svg className={`branch-icon ${className ?? ""}`} width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
      <circle cx="3" cy="2.2" r="1.5" />
      <circle cx="3" cy="9.8" r="1.5" />
      <circle cx="9" cy="3.8" r="1.5" />
      <path d="M3 3.7v4.6M9 5.3c0 2.2-1.6 2.8-4.6 3.6" fill="none" />
    </svg>
  );
}
