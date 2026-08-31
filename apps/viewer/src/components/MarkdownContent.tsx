import ReactMarkdown, { type Components, type UrlTransform } from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

interface MarkdownContentProps {
  content: string;
  class_name?: string;
}

const ALLOWED_ELEMENTS = [
  "a",
  "blockquote",
  "br",
  "code",
  "del",
  "em",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "hr",
  "img",
  "input",
  "li",
  "ol",
  "p",
  "pre",
  "strong",
  "table",
  "tbody",
  "td",
  "th",
  "thead",
  "tr",
  "ul",
] as const;

const SAFE_LINK_PROTOCOLS = new Set(["http:", "https:"]);

const safeUrlTransform: UrlTransform = (url) => {
  try {
    const parsed = new URL(url);
    return SAFE_LINK_PROTOCOLS.has(parsed.protocol) ? parsed.href : "";
  } catch {
    return "";
  }
};

const MARKDOWN_COMPONENTS = {
  a({ children }) {
    return <span className="markdown-content__link">{children}</span>;
  },
  img({ alt }) {
    const label = alt ? `Image: ${alt}` : "Image";
    return (
      <span aria-label={label} className="markdown-content__image" role="img">
        {alt ? `[Image: ${alt}]` : "[Image]"}
      </span>
    );
  },
  input({ checked }) {
    return (
      <input
        aria-label={checked ? "Completed task" : "Incomplete task"}
        checked={checked ?? false}
        disabled
        readOnly
        type="checkbox"
      />
    );
  },
  table({ children }) {
    return (
      <div className="markdown-content__table-wrap">
        <table>{children}</table>
      </div>
    );
  },
} satisfies Components;

export function MarkdownContent({ content, class_name }: MarkdownContentProps) {
  const rootClassName = ["markdown-content", class_name].filter(Boolean).join(" ");

  return (
    <div className={rootClassName}>
      <ReactMarkdown
        allowedElements={ALLOWED_ELEMENTS}
        components={MARKDOWN_COMPONENTS}
        remarkPlugins={[remarkGfm, remarkBreaks]}
        skipHtml
        urlTransform={safeUrlTransform}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
