import { afterEach, expect, it } from "vitest";
import { consumeLoginToken } from "./login";

afterEach(() => window.history.replaceState(null, "", "/"));

it("consumes only the fragment token and removes it before returning", () => {
  window.history.replaceState(null, "", "/?filter=pi#token=secret%2Bvalue&panel=sessions");
  expect(consumeLoginToken()).toBe("secret+value");
  expect(window.location.search).toBe("?filter=pi");
  expect(window.location.hash).toBe("#panel=sessions");
  expect(consumeLoginToken()).toBeUndefined();
});

it("does not interpret query parameters as login credentials", () => {
  window.history.replaceState(null, "", "/?token=query-secret");
  expect(consumeLoginToken()).toBeUndefined();
});
