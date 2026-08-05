export interface SequencedTransportOutput {
  data: number[];
  sequence: number;
}

export type TransportOutputPayload = number[] | SequencedTransportOutput;

export interface TransportOutput {
  bytes: Uint8Array;
  sequence: number | null;
}

export function readTransportOutput(payload: TransportOutputPayload): TransportOutput {
  if (Array.isArray(payload)) {
    return {
      bytes: new Uint8Array(payload),
      sequence: null,
    };
  }
  return {
    bytes: new Uint8Array(payload.data),
    sequence: payload.sequence,
  };
}
