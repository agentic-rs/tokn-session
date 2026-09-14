export function DetailRefreshError({ error, on_retry }: {
  error: string | null;
  on_retry: () => void;
}) {
  if (!error) return null;
  return (
    <div className="detail-refresh-error" role="alert">
      <span>Could not refresh: {error}</span>
      <button className="text-button" onClick={on_retry} type="button">Try again</button>
    </div>
  );
}
