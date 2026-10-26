import { z } from "zod";

/** Bridge protocol version (docs/bridge-protocol.md). */
export const BRIDGE_PROTO = 1 as const;

export const RendezvousSpec = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("gist"), gist_id: z.string().min(1) }),
]);

/** What the launcher embeds in the kernel it pushes. It contains no secrets. */
export const SessionTicket = z.object({
  proto: z.literal(BRIDGE_PROTO),
  session_id: z.string().regex(/^[0-9a-f]{32}$/, "expected 32 lowercase hex chars"),
  /** The app's X25519 public key, standard base64. */
  app_pk: z.base64(),
  /** Seconds after boot when the worker stops itself. */
  deadline_s: z.number().int().positive(),
  /** Dead-man switch: seconds without an authenticated request before the worker stops. */
  idle_s: z.number().int().positive(),
  rendezvous: RendezvousSpec,
});

export type RendezvousSpec = z.infer<typeof RendezvousSpec>;
export type SessionTicket = z.infer<typeof SessionTicket>;
