# Skald protocol: test vectors

Part of the [Skald protocol](README.md), with its conventions and status. **No external audit.**

- Each vector is one CBOR item: its value in diagnostic notation [[RFC 8949](https://www.rfc-editor.org/rfc/rfc8949), §8] and its bytes in hex. `<< >>` marks a byte string that holds an encoded item [[RFC 8610](https://www.rfc-editor.org/rfc/rfc8610), Appendix G.3].
- An implementation encodes each value to exactly these bytes and decodes the bytes back to the value.
- Signatures, keys, ciphertexts and the MLS payload are placeholders: the vectors fix the encoding, not the cryptography.
- Device IDs are derived from the public keys of [[RFC 8032](https://www.rfc-editor.org/rfc/rfc8032)], §7.1: Alice's device from "TEST 1", Bob's from "TEST 2", the device of the service grant and of the delegation from "TEST 3".
- The vectors cover each [envelope scheme](README.md#envelope), the [HPKE plaintext](README.md#scheme-hpke), every optional field of the header and the [content](README.md#content), and the receipts: the `receipt` body for `held`, `refused` and `read`, the server's `stored` frame and its `delivery-status` for `delivered` and `expired`, and each other [frame](delivery.md#frames). `replied` is a reply, content with `reply_to`, as in [Content with an A2A message and an attachment](#content-with-an-a2a-message-and-an-attachment).

## Contents
- [Envelope under `mls`](#envelope-under-mls)
- [Envelope under `hpke`](#envelope-under-hpke)
- [HPKE plaintext](#hpke-plaintext)
- [Content with an A2A message and an attachment](#content-with-an-a2a-message-and-an-attachment)
- [Content from a service device](#content-from-a-service-device)
- [Content from a custodian device](#content-from-a-custodian-device)
- [Receipt `held`](#receipt-held)
- [Receipt `refused`](#receipt-refused)
- [Receipt `read`](#receipt-read)
- [Server status `stored`](#server-status-stored)
- [Server status `delivered`](#server-status-delivered)
- [Server status `expired`](#server-status-expired)
- [Frame `send`](#frame-send)
- [Frame `send-refused`](#frame-send-refused)
- [Frame `sync`](#frame-sync)
- [Frame `message`](#frame-message)
- [Frame `status`](#frame-status)
- [Frame `ack`](#frame-ack)
- [Frame `batch`](#frame-batch)
- [Frame `space-notice`](#frame-space-notice)

## Envelope under `mls`

```cbor-diag
[
  {
    1: 1,
    2: h'019a6f1e2b3c7d4e8f5061a2b3c4d5e6',
    3: h'019a6f1e2b3c7d4e8f5061a2b3c4d5e7',
    4: h'019a6f201a007c008a00000000000001',
    5: 1
  },
  h'000100020102030405060708090a0b0c0d0e0f'
]
```

```hex
82a501010250019a6f1e2b3c7d4e8f5061a2b3c4d5e60350019a6f1e2b3c7d4e
8f5061a2b3c4d5e70450019a6f201a007c008a00000000000001050153000100
020102030405060708090a0b0c0d0e0f
```

## Envelope under `hpke`

```cbor-diag
[
  {
    1: 1,
    2: h'019a6f1e2b3c7d4e8f5061a2b3c4d5e6',
    3: h'019a6f1e2b3c7d4e8f5061a2b3c4d5e7',
    4: h'019a6f201a007c008a00000000000002',
    5: 2,
    6: [
      h'188e6ae45b2d8247b1e1583b84bd49c7'
    ],
    7: 86400
  },
  <<[
    h'e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0',
    h'c0c1c2c3c4c5c6c7c8c9cacbcccdcecf'
  ]>>
]
```

```hex
82a701010250019a6f1e2b3c7d4e8f5061a2b3c4d5e60350019a6f1e2b3c7d4e
8f5061a2b3c4d5e70450019a6f201a007c008a00000000000002050206815018
8e6ae45b2d8247b1e1583b84bd49c7071a000151805834825820e0e0e0e0e0e0
e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e050c0c1c2c3c4
c5c6c7c8c9cacbcccdcecf
```

## HPKE plaintext

```cbor-diag
[
  h'19ac5f784938823586a6c5809a18f6dd',
  <<{
    1: "acme/alice/claude-api@skald.example",
    2: "acme/bob/codex@skald.example",
    3: 1,
    4: 0,
    6: "{\"messageId\":\"019a6f20-1a00-7c00-8a00-000000000002\",\"contextId\":\"019a6f20-0000-7000-8000-00000000c0de\",\"role\":\"ROLE_USER\",\"parts\":[{\"text\":\"Review PR 12\"}]}"
  }>>,
  h'5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a'
]
```

```hex
835019ac5f784938823586a6c5809a18f6dd58e9a501782361636d652f616c69
63652f636c617564652d61706940736b616c642e6578616d706c6502781c6163
6d652f626f622f636f64657840736b616c642e6578616d706c65030104000678
9c7b226d6573736167654964223a2230313961366632302d316130302d376330
302d386130302d303030303030303030303032222c22636f6e74657874496422
3a2230313961366632302d303030302d373030302d383030302d303030303030
303063306465222c22726f6c65223a22524f4c455f55534552222c2270617274
73223a5b7b2274657874223a22526576696577205052203132227d5d7d58405a
5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a
5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a
```

## Content with an A2A message and an attachment

```cbor-diag
{
  1: "acme/alice/claude-api@skald.example",
  2: "acme/bob/codex@skald.example",
  3: 7,
  4: 1,
  5: h'019a6f201a007c008a00000000000003',
  6: "{\"messageId\":\"019a6f20-1a00-7c00-8a00-000000000004\",\"contextId\":\"019a6f20-0000-7000-8000-00000000c0de\",\"role\":\"ROLE_USER\",\"parts\":[{\"text\":\"Here is the diff\"},{\"url\":\"urn:uuid:019a6f20-1b00-7d00-8b00-00000000b10b\",\"filename\":\"pr-12.diff\",\"mediaType\":\"text/x-diff\"}]}",
  10: [
    {
      1: h'019a6f201b007d008b0000000000b10b',
      2: h'6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b',
      3: h'd1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1',
      4: 1234
    }
  ]
}
```

```hex
a701782361636d652f616c6963652f636c617564652d61706940736b616c642e
6578616d706c6502781c61636d652f626f622f636f64657840736b616c642e65
78616d706c65030704010550019a6f201a007c008a000000000000030679010a
7b226d6573736167654964223a2230313961366632302d316130302d37633030
2d386130302d303030303030303030303034222c22636f6e746578744964223a
2230313961366632302d303030302d373030302d383030302d30303030303030
3063306465222c22726f6c65223a22524f4c455f55534552222c227061727473
223a5b7b2274657874223a2248657265206973207468652064696666227d2c7b
2275726c223a2275726e3a757569643a30313961366632302d316230302d3764
30302d386230302d303030303030303062313062222c2266696c656e616d6522
3a2270722d31322e64696666222c226d6564696154797065223a22746578742f
782d64696666227d5d7d0a81a40150019a6f201b007d008b0000000000b10b02
506b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b035820d1d1d1d1d1d1d1d1d1d1d1d1
d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1041904d2
```

## Content from a service device

```cbor-diag
{
  1: "acme/ci/deploy@skald.example",
  4: 0,
  6: "{\"messageId\":\"019a6f20-1a00-7c00-8a00-000000000005\",\"contextId\":\"019a6f20-0000-7000-8000-00000000c0de\",\"role\":\"ROLE_USER\",\"parts\":[{\"text\":\"Deployed\"}]}",
  8: [
    <<{
      1: h'42592ce391c8869ebdf88dff1cbfc25a',
      2: [
        h'019a6f1e2b3c7d4e8f5061a2b3c4d5e7',
        h'019a6f1e2b3c7d4e8f5061a2b3c4d5e8'
      ],
      3: h'19ac5f784938823586a6c5809a18f6dd',
      4: 1798761600
    }>>,
    h'5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a'
  ]
}
```

```hex
a401781c61636d652f63692f6465706c6f7940736b616c642e6578616d706c65
04000678987b226d6573736167654964223a2230313961366632302d31613030
2d376330302d386130302d303030303030303030303035222c22636f6e746578
744964223a2230313961366632302d303030302d373030302d383030302d3030
30303030303063306465222c22726f6c65223a22524f4c455f55534552222c22
7061727473223a5b7b2274657874223a224465706c6f796564227d5d7d088258
4fa4015042592ce391c8869ebdf88dff1cbfc25a028250019a6f1e2b3c7d4e8f
5061a2b3c4d5e750019a6f1e2b3c7d4e8f5061a2b3c4d5e8035019ac5f784938
823586a6c5809a18f6dd041a6b36ec8058405a5a5a5a5a5a5a5a5a5a5a5a5a5a
5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a
5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a
```

## Content from a custodian device

```cbor-diag
{
  1: "acme/alice/hosted-mcp@skald.example",
  3: 3,
  4: 0,
  6: "{\"messageId\":\"019a6f20-1a00-7c00-8a00-000000000006\",\"contextId\":\"019a6f20-0000-7000-8000-00000000c0de\",\"role\":\"ROLE_USER\",\"parts\":[{\"text\":\"Done\"}]}",
  7: [
    <<{
      1: "acme/alice/hosted-mcp@skald.example",
      2: h'42592ce391c8869ebdf88dff1cbfc25a',
      3: h'19ac5f784938823586a6c5809a18f6dd',
      4: 1798761600
    }>>,
    h'5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a'
  ]
}
```

```hex
a501782361636d652f616c6963652f686f737465642d6d637040736b616c642e
6578616d706c65030304000678947b226d6573736167654964223a2230313961
366632302d316130302d376330302d386130302d303030303030303030303036
222c22636f6e746578744964223a2230313961366632302d303030302d373030
302d383030302d303030303030303063306465222c22726f6c65223a22524f4c
455f55534552222c227061727473223a5b7b2274657874223a22446f6e65227d
5d7d07825851a401782361636d652f616c6963652f686f737465642d6d637040
736b616c642e6578616d706c65025042592ce391c8869ebdf88dff1cbfc25a03
5019ac5f784938823586a6c5809a18f6dd041a6b36ec8058405a5a5a5a5a5a5a
5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a
5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a
```

## Receipt `held`

```cbor-diag
{
  1: "acme/bob/codex@skald.example",
  2: "acme/alice/claude-api@skald.example",
  3: 5,
  4: 0,
  9: {
    1: 1,
    2: [
      h'019a6f201a007c008a00000000000004'
    ]
  }
}
```

```hex
a501781c61636d652f626f622f636f64657840736b616c642e6578616d706c65
02782361636d652f616c6963652f636c617564652d61706940736b616c642e65
78616d706c650305040009a20101028150019a6f201a007c008a000000000000
04
```

## Receipt `refused`

```cbor-diag
{
  1: "acme/bob/codex@skald.example",
  2: "acme/alice/claude-api@skald.example",
  3: 5,
  4: 0,
  9: {
    1: 2,
    2: [
      h'019a6f201a007c008a00000000000004'
    ],
    3: "Inbound mode is refuse"
  }
}
```

```hex
a501781c61636d652f626f622f636f64657840736b616c642e6578616d706c65
02782361636d652f616c6963652f636c617564652d61706940736b616c642e65
78616d706c650305040009a30102028150019a6f201a007c008a000000000000
040376496e626f756e64206d6f646520697320726566757365
```

## Receipt `read`

```cbor-diag
{
  1: "acme/bob/codex@skald.example",
  2: "acme/alice/claude-api@skald.example",
  3: 5,
  4: 0,
  9: {
    1: 3,
    2: [
      h'019a6f201a007c008a00000000000001',
      h'019a6f201a007c008a00000000000004'
    ]
  }
}
```

```hex
a501781c61636d652f626f622f636f64657840736b616c642e6578616d706c65
02782361636d652f616c6963652f636c617564652d61706940736b616c642e65
78616d706c650305040009a20103028250019a6f201a007c008a000000000000
0150019a6f201a007c008a00000000000004
```

## Server status `stored`

```cbor-diag
[
  2,
  [
    h'019a6f201a007c008a00000000000002',
    h'188e6ae45b2d8247b1e1583b84bd49c7'
  ],
  1762819200
]
```

```hex
83028250019a6f201a007c008a0000000000000250188e6ae45b2d8247b1e158
3b84bd49c71a69127c80
```

## Server status `delivered`

```cbor-diag
{
  1: h'019a6f201a007c008a00000000000001',
  2: h'188e6ae45b2d8247b1e1583b84bd49c7',
  3: 1,
  4: 1762819200
}
```

```hex
a40150019a6f201a007c008a000000000000010250188e6ae45b2d8247b1e158
3b84bd49c70301041a69127c80
```

## Server status `expired`

```cbor-diag
{
  1: h'019a6f201a007c008a00000000000001',
  2: h'188e6ae45b2d8247b1e1583b84bd49c7',
  3: 2,
  4: 1762819200
}
```

```hex
a40150019a6f201a007c008a000000000000010250188e6ae45b2d8247b1e158
3b84bd49c70302041a69127c80
```

## Frame `send`

```cbor-diag
[
  1,
  <<[
    {
      1: 1,
      2: h'019a6f1e2b3c7d4e8f5061a2b3c4d5e6',
      3: h'019a6f1e2b3c7d4e8f5061a2b3c4d5e7',
      4: h'019a6f201a007c008a00000000000001',
      5: 1
    },
    h'000100020102030405060708090a0b0c0d0e0f'
  ]>>,
  true
]
```

```hex
8301585082a501010250019a6f1e2b3c7d4e8f5061a2b3c4d5e60350019a6f1e
2b3c7d4e8f5061a2b3c4d5e70450019a6f201a007c008a000000000000010501
53000100020102030405060708090a0b0c0d0e0ff5
```

## Frame `send-refused`

```cbor-diag
[
  3,
  [
    h'019a6f201a007c008a00000000000002',
    h'188e6ae45b2d8247b1e1583b84bd49c7'
  ],
  "unknown-recipient"
]
```

```hex
83038250019a6f201a007c008a0000000000000250188e6ae45b2d8247b1e158
3b84bd49c771756e6b6e6f776e2d726563697069656e74
```

## Frame `sync`

```cbor-diag
[
  4,
  7
]
```

```hex
820407
```

## Frame `message`

```cbor-diag
[
  5,
  8,
  <<[
    {
      1: 1,
      2: h'019a6f1e2b3c7d4e8f5061a2b3c4d5e6',
      3: h'019a6f1e2b3c7d4e8f5061a2b3c4d5e7',
      4: h'019a6f201a007c008a00000000000002',
      5: 2,
      6: [
        h'188e6ae45b2d8247b1e1583b84bd49c7'
      ],
      7: 86400
    },
    <<[
      h'e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0',
      h'c0c1c2c3c4c5c6c7c8c9cacbcccdcecf'
    ]>>
  ]>>
]
```

```hex
830508588b82a701010250019a6f1e2b3c7d4e8f5061a2b3c4d5e60350019a6f
1e2b3c7d4e8f5061a2b3c4d5e70450019a6f201a007c008a0000000000000205
02068150188e6ae45b2d8247b1e1583b84bd49c7071a000151805834825820e0
e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e050
c0c1c2c3c4c5c6c7c8c9cacbcccdcecf
```

## Frame `status`

```cbor-diag
[
  6,
  9,
  {
    1: h'019a6f201a007c008a00000000000001',
    2: h'188e6ae45b2d8247b1e1583b84bd49c7',
    3: 1,
    4: 1762819200
  }
]
```

```hex
830609a40150019a6f201a007c008a000000000000010250188e6ae45b2d8247
b1e1583b84bd49c70301041a69127c80
```

## Frame `ack`

```cbor-diag
[
  7,
  9
]
```

```hex
820709
```

## Frame `batch`

```cbor-diag
[
  8,
  [
    [
      6,
      10,
      {
        1: h'019a6f201a007c008a00000000000001',
        2: h'188e6ae45b2d8247b1e1583b84bd49c7',
        3: 2,
        4: 1762819200
      }
    ],
    [
      9,
      h'019a6f1e2b3c7d4e8f5061a2b3c4d5e6'
    ]
  ]
]
```

```hex
82088283060aa40150019a6f201a007c008a000000000000010250188e6ae45b
2d8247b1e1583b84bd49c70302041a69127c80820950019a6f1e2b3c7d4e8f50
61a2b3c4d5e6
```

## Frame `space-notice`

```cbor-diag
[
  9,
  h'019a6f1e2b3c7d4e8f5061a2b3c4d5e6'
]
```

```hex
820950019a6f1e2b3c7d4e8f5061a2b3c4d5e6
```
