import { afterEach, expect, it, vi } from "vitest";
import { creationOptions, credentialJson, decodeBase64Url, detectHub, encodeBase64Url, HubClient, requestOptions } from "./hub";

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it("converts WebAuthn binary fields without altering policy and authenticator hints", () => {
  const options = creationOptions({
    challenge: "_wAB",
    rp: { name: "Tokn Hub", id: "hub.example" },
    user: { id: "AQI", name: "owner", displayName: "Owner" },
    pubKeyCredParams: [{ type: "public-key", alg: -7 }],
    excludeCredentials: [{ id: "AwQ", type: "public-key", transports: ["internal"] }],
    authenticatorSelection: { userVerification: "required", residentKey: "required" },
  });
  expect(Array.from(new Uint8Array(options.challenge as ArrayBuffer))).toEqual([255, 0, 1]);
  expect(Array.from(new Uint8Array(options.user.id as ArrayBuffer))).toEqual([1, 2]);
  expect(options.excludeCredentials?.[0].transports).toEqual(["internal"]);
  expect(options.authenticatorSelection?.userVerification).toBe("required");
  expect(encodeBase64Url(decodeBase64Url("-_8"))).toBe("-_8");
  const request = requestOptions({ challenge: "AQI", allowCredentials: [{ id: "AwQ", type: "public-key" }], userVerification: "required" });
  expect(Array.from(new Uint8Array(request.allowCredentials![0].id as ArrayBuffer))).toEqual([3, 4]);
  expect(request.userVerification).toBe("required");
});

function assertion(): PublicKeyCredential {
  return {
    id: "AQI", rawId: decodeBase64Url("AQI"), type: "public-key", authenticatorAttachment: "platform",
    getClientExtensionResults: () => ({}),
    response: { clientDataJSON: decodeBase64Url("AwQ"), authenticatorData: decodeBase64Url("BQY"), signature: decodeBase64Url("Bwg"), userHandle: null },
  } as unknown as PublicKeyCredential;
}

it("serializes both WebAuthn responses using the server's credential wire format", () => {
  expect(credentialJson(assertion())).toEqual({
    id: "AQI", rawId: "AQI", type: "public-key", authenticatorAttachment: "platform", extensions: {},
    response: { clientDataJSON: "AwQ", authenticatorData: "BQY", signature: "Bwg", userHandle: null },
  });
  const attestation = assertion();
  Object.defineProperty(attestation, "response", { value: {
    clientDataJSON: decodeBase64Url("AwQ"), attestationObject: decodeBase64Url("BQY"), getTransports: () => ["hybrid"],
  } });
  expect(credentialJson(attestation).response).toEqual({ clientDataJSON: "AwQ", attestationObject: "BQY", transports: ["hybrid"] });
});

it("submits a real credential ceremony and keeps the session credential only in memory", async () => {
  vi.stubGlobal("PublicKeyCredential", class {});
  const get = vi.fn().mockResolvedValue(assertion());
  vi.stubGlobal("navigator", { credentials: { get } });
  const fetch = vi.spyOn(globalThis, "fetch")
    .mockResolvedValueOnce(Response.json({ ceremony_id: "ceremony", options: { publicKey: { challenge: "AQI", rpId: "hub.example", userVerification: "required" } } }))
    .mockResolvedValueOnce(Response.json({ access_token: "session-secret", token_type: "Bearer", expires_in: 3600 }));
  const storage = vi.spyOn(Storage.prototype, "setItem");
  const client = new HubClient();
  const session = await client.authenticate(false);
  expect(get.mock.calls[0][0].publicKey.rpId).toBe("hub.example");
  expect(JSON.parse(fetch.mock.calls[1][1]!.body as string)).toEqual({ ceremony_id: "ceremony", credential: credentialJson(assertion()) });
  expect(session.access_token).toBe("session-secret");
  expect(storage).not.toHaveBeenCalled();
  session.close();
  expect(session.access_token).toBe("");
  expect(session.signal.aborted).toBe(true);
  client.close();
});

it("distinguishes a direct SPA fallback from Hub errors and aborts authenticated requests on close", async () => {
  const fetch = vi.spyOn(globalThis, "fetch")
    .mockResolvedValueOnce(new Response("<html></html>", { headers: { "Content-Type": "text/html" } }))
    .mockResolvedValueOnce(new Response(null, { status: 503 }));
  expect(await detectHub(new AbortController().signal)).toBeNull();
  await expect(detectHub(new AbortController().signal)).rejects.toThrow("503");
  fetch.mockImplementationOnce((_url, options) => new Promise((_resolve, reject) => {
    options!.signal!.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")));
  }));
  const client = new HubClient("secret");
  const pending = client.request("hosts");
  const request = fetch.mock.calls[2][1]!;
  expect(request.headers).toMatchObject({ Authorization: "Bearer secret" });
  expect(request.redirect).toBe("error");
  client.close();
  await expect(pending).rejects.toThrow("Aborted");
});
