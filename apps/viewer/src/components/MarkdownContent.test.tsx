import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { MarkdownContent } from "./MarkdownContent";

afterEach(cleanup);

describe("MarkdownContent", () => {
  it("renders GFM tables, strikethrough, and task lists", () => {
    const { container } = render(
      <MarkdownContent
        content={[
          "~~removed~~",
          "",
          "| Name | State |",
          "| --- | --- |",
          "| viewer | ready |",
          "",
          "- [x] Render Markdown",
        ].join("\n")}
      />,
    );

    expect(screen.getByText("removed").tagName).toBe("DEL");
    expect(screen.getByRole("table")).toHaveTextContent("viewer");
    expect(screen.getByRole("table").parentElement).toHaveClass(
      "markdown-content__table-wrap",
    );
    expect(screen.getByRole("checkbox", { name: "Completed task" })).toBeChecked();
    expect(container.firstElementChild).toHaveClass("markdown-content");
  });

  it("renders fenced and inline code without interpreting either as HTML", () => {
    const { container } = render(
      <MarkdownContent content={"Use `<safe>` inline.\n\n```ts\nconst value = 1;\n```"} />,
    );
    const code = container.querySelectorAll("code");

    expect(code).toHaveLength(2);
    expect(code[0]).toHaveTextContent("<safe>");
    expect(code[0].parentElement?.tagName).toBe("P");
    expect(code[1]).toHaveClass("language-ts");
    expect(code[1].parentElement?.tagName).toBe("PRE");
    expect(container.querySelector("safe")).not.toBeInTheDocument();
  });

  it("turns soft line breaks into visible break elements", () => {
    const { container } = render(<MarkdownContent content={"first line\nsecond line"} />);
    const paragraph = container.querySelector("p");

    expect(paragraph).toHaveTextContent("first line second line");
    expect(paragraph?.querySelector("br")).toBeInTheDocument();
  });

  it("drops raw HTML and never creates scriptable elements", () => {
    const { container } = render(
      <MarkdownContent
        content={'before<script src="https://example.test/run.js">alert(1)</script><iframe src="https://example.test"></iframe>after'}
      />,
    );

    expect(container.querySelector("script")).not.toBeInTheDocument();
    expect(container.querySelector("iframe")).not.toBeInTheDocument();
    expect(container.innerHTML).not.toContain("run.js");
  });

  it("renders links as inert text and rejects unsafe or custom destinations", () => {
    const { container } = render(
      <MarkdownContent
        content={[
          "[web](https://example.com)",
          "[script](javascript:alert(1))",
          "[data](data:text/html,unsafe)",
          "[custom](codex://task/123)",
        ].join(" ")}
      />,
    );

    expect(container.querySelectorAll(".markdown-content__link")).toHaveLength(4);
    expect(container.querySelector("a")).not.toBeInTheDocument();
    expect(container.querySelector("[href]")).not.toBeInTheDocument();
    expect(container.innerHTML).not.toMatch(/javascript:|data:text|codex:/i);
  });

  it("replaces images with accessible placeholders without loading a URL", () => {
    const { container } = render(
      <MarkdownContent
        content={'![architecture](https://example.com/tracker.png "diagram") ![](data:image/svg+xml,unsafe)'}
      />,
    );

    expect(container.querySelector("img")).not.toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Image: architecture" })).toHaveClass(
      "markdown-content__image",
    );
    expect(screen.getByRole("img", { name: "Image" })).toHaveTextContent("[Image]");
    expect(container.innerHTML).not.toMatch(/tracker\.png|data:image/i);
  });
});
