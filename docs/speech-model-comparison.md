# Speech model comparison

Status: **plan and placeholder. No model has been measured yet.** The team recordings needed for the test set do not exist in the repo and must not be added to it (hard rule: no real meeting data).

## Assumption (logged, not measured)

Until the numbers below exist, the default is `large-v3-turbo` (`speech_model` in `config.example.toml`). Reason: it is the cheapest to run on one GPU and fast enough for the live view. This is an assumption to confirm.

## Shortlist

| Model | What we expect | Risk |
|---|---|---|
| `large-v3` | Best stock accuracy on Vietnamese; slowest, most VRAM | Repeats or invents text on silence |
| `large-v3-turbo` | Much faster decoding, smaller memory; Vietnamese gap to `large-v3` not known | May lose accuracy on tones and code-switching |
| A Vietnamese fine-tune converted to ggml | Possibly lowest Vietnamese error | English and code-switching may get worse; timestamps may drift; licence must be checked |

`medium` is dropped: expected to be clearly worse on Vietnamese tones.
These expectations come from general knowledge and have not been verified here.

## Known behaviour to design for

- One language is assumed per 30 s window, so mixed vi+en needs `language=vi` plus a team glossary in the initial prompt; compare against auto-detect.
- Silence causes hallucination. Use voice-activity detection first, no carried context, a no-speech threshold, and drop segments with a very high compression ratio.

## Evaluation protocol

1. **Test set:** 3 to 5 hours, about 20 to 30 consented meetings with varied speakers and microphones, hand-corrected. Stored outside the repo. Add public Vietnamese sets as sanity checks.
2. **Normalisation (same for reference and output):** Unicode NFC, lowercase, strip punctuation, normalise tone-mark placement, spell out numbers.
3. **Metrics** (from the `eval` WER binary): WER and CER; WER with diacritics stripped to isolate tone errors; WER on English tokens in code-switched spans. Vietnamese WER is effectively a syllable error rate.
4. **Silence test:** 30+ minutes of silence and noise; count words inserted per minute.
5. **Speed:** real-time factor and peak VRAM on the production GPU.

## Results

| Model | WER | CER | WER no-diacritics | English-span WER | Insertions/min | RTF | VRAM |
|---|---|---|---|---|---|---|---|
| `large-v3` | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| `large-v3-turbo` | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| Vietnamese fine-tune | TBD | TBD | TBD | TBD | TBD | TBD | TBD |

## Needed from the team

Consent and 20 to 30 recordings, plus corrected transcripts. Decision rule: pick the lowest WER that stays within the speed budget; switch to the fine-tune only if it wins on code-switched clips, not just monolingual ones.
