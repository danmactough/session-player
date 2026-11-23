/** Saves `blob` as a browser download named `filename`; returns the name. */
export function downloadBlob({ blob, filename }: { blob: Blob; filename: string }): string {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.append(a);
  a.click();
  a.remove();
  // The download has its own copy once it starts; revoking at once can cancel it in some browsers.
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
  return filename;
}
