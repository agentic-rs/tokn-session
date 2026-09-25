import "@testing-library/jest-dom/vitest";

// jsdom has no viewport layout or native dialog implementation.
Object.defineProperty(window, "matchMedia", {
  configurable: true,
  writable: true,
  value: (media: string) => ({
    matches: false,
    media,
    addEventListener() {},
    removeEventListener() {},
  }),
});
Object.defineProperties(HTMLDialogElement.prototype, {
  showModal: {
    configurable: true,
    value() { this.setAttribute("open", ""); },
  },
  close: {
    configurable: true,
    value() { this.removeAttribute("open"); },
  },
});
