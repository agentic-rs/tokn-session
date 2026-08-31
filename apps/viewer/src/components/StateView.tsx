import { WarningIcon } from "./Icons";

interface StateViewProps {
  title: string;
  message: string;
  action_label?: string;
  on_action?: () => void;
  tone?: "neutral" | "error";
}

export function StateView({
  title,
  message,
  action_label,
  on_action,
  tone = "neutral",
}: StateViewProps) {
  return (
    <div className={`state-view state-view--${tone}`} role={tone === "error" ? "alert" : "status"}>
      <span className="state-view__icon">
        <WarningIcon />
      </span>
      <h2>{title}</h2>
      <p>{message}</p>
      {action_label && on_action ? (
        <button className="secondary-button" onClick={on_action} type="button">
          {action_label}
        </button>
      ) : null}
    </div>
  );
}

export function LoadingRows({ count = 5 }: { count?: number }) {
  return (
    <div aria-label="Loading" className="loading-rows" role="status">
      {Array.from({ length: count }, (_, index) => (
        <div className="loading-row" key={index}>
          <span className="skeleton loading-row__dot" />
          <span className="loading-row__copy">
            <span className="skeleton loading-row__title" />
            <span className="skeleton loading-row__detail" />
          </span>
        </div>
      ))}
    </div>
  );
}
