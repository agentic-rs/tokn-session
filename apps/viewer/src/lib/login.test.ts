import { afterEach, expect, it } from "vitest";
import { consumeBootstrapToken, consumeLoginToken } from "./login";

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

it("removes Hub bootstrap credentials before rendering while preserving unrelated fragments", () => {
  window.history.replaceState(null, "", "/#bootstrap_token=setup%2Bsecret&panel=hosts");
  expect(consumeBootstrapToken()).toBe("setup+secret");
  expect(window.location.hash).toBe("#panel=hosts");
  expect(consumeBootstrapToken()).toBeUndefined();
});
