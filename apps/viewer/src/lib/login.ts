/** Consume login credentials before React renders (including StrictMode remounts). */
export function consumeLoginToken(): string | undefined {
  const fragment = new URLSearchParams(window.location.hash.slice(1));
  if (!fragment.has("token")) return undefined;
  const token = fragment.get("token") || undefined;
  fragment.delete("token");
  const remaining = fragment.toString();
  window.history.replaceState(window.history.state, "", `${window.location.pathname}${window.location.search}${remaining ? `#${remaining}` : ""}`);
  return token;
}
