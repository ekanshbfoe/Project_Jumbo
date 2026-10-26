import { describe, expect, it } from "vitest";
import { LedgerEvent, SessionTicket } from "../src";

const hash = "a".repeat(64);
const event = {
  seq: 0,
  commit: 0,
  part: 0,
  parts: 1,
  ts: "2026-10-19T12:00:00.123456789Z",
  chapter_id: "ch_0001",
  op: { op: "NOOP" },
  prev: "0".repeat(64),
  hash,
};

describe("LedgerEvent", () => {
  it("accepts an event without a session", () => {
    expect(LedgerEvent.parse(event).session).toBeUndefined();
  });

  it("rejects a part outside its commit and malformed hashes", () => {
    expect(LedgerEvent.safeParse({ ...event, part: 1 }).success).toBe(false);
    expect(LedgerEvent.safeParse({ ...event, hash: "ABC" }).success).toBe(false);
    expect(LedgerEvent.safeParse({ ...event, ts: "yesterday" }).success).toBe(false);
  });
});

describe("SessionTicket", () => {
  const ticket = {
    proto: 1,
    session_id: "0123456789abcdef0123456789abcdef",
    app_pk: "j0DFrbaPJWJK5bIU6nZ6bslNgp09e14a0bpvPiE4KF8=",
    deadline_s: 30600,
    idle_s: 1800,
    rendezvous: { kind: "gist", gist_id: "g" },
  };

  it("rejects other protocol versions and odd session ids", () => {
    expect(SessionTicket.safeParse(ticket).success).toBe(true);
    expect(SessionTicket.safeParse({ ...ticket, proto: 2 }).success).toBe(false);
    expect(SessionTicket.safeParse({ ...ticket, session_id: "xyz" }).success).toBe(false);
    expect(SessionTicket.safeParse({ ...ticket, app_pk: "not base64!" }).success).toBe(false);
  });
});
