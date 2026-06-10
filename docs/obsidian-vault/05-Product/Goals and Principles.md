---
tags:
  - product
  - goals
---

# Goals and Principles

## Product Goal

Make AV1 transcoding safe enough and easy enough that a user can reduce a large video library without babysitting ffmpeg, risking originals, or manually tracking which files are done.

## User Promise

- "Drop work into the system and come back to verified `_av1` files."
- "The original stays safe until I choose what to delete."
- "I can see what is happening and pause it when needed."

## Non-Goals For Limited Release

- Public multi-tenant SaaS.
- Arbitrary internet exposure.
- Mobile worker support.
- Full cross-platform parity.
- Automatic destructive source replacement.
- Perceptual quality scoring beyond basic release safety, unless already cheap to implement.

## Product Principles

- Prefer explicit states over hidden assumptions.
- Never let `done` mean "probably okay".
- Make pause/resume obvious.
- Make recovery boring.
- Keep workers disposable.
- Keep release scope small.
- Keep secrets out of the repo.

## Success Signals

- A trusted user can install the companion, submit a file, and receive a verified output without developer help.
- A worker reboot does not lose or duplicate a job.
- The dashboard shows enough state to answer: what is running, what is pending, what failed, and what is safe to delete.
- A release operator can update queue, worker, and companion using written steps.

