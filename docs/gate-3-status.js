/**
 * Applies the current Gate 3 delivery status to the narrative planning pages.
 *
 * The structured sources remain `roadmap.json` and `gate-3-acceptance.json`.
 * This small progressive enhancement avoids rewriting the older, deliberately
 * self-contained planning HTML while work is still being qualified.
 */
function updateDocumentationIndex() {
  const gate3 = document.querySelector('#gates .card:nth-child(3)');
  if (!gate3) return;

  const status = gate3.querySelector('.label span');
  if (status) {
    status.className = 'open';
    status.textContent = 'locally qualified; live gaps remain';
  }

  const detail = gate3.querySelector('p');
  if (detail) {
    detail.textContent =
      'The durable session/context/availability/artifact model, native ' +
      'UCTP/QUIC audio, local SIP and authenticated WebRTC, managed files, ' +
      'language SDKs and recovery have passing local evidence. ' +
      'Human-audibility, answered/inbound PSTN and interactive browser ' +
      'proofs remain open. The current local package passed installation checks.';
  }
}

function updateGate3Plan() {
  if (!document.querySelector('title')?.textContent.includes('Gate 3')) return;

  const pills = document.querySelectorAll('.hero .pill');
  if (pills[3]) {
    pills[3].textContent = 'Local qualification passed · live gaps remain';
  }

  const heroNote = document.querySelector('.hero .small');
  if (heroNote) {
    heroNote.innerHTML =
      'Six decision directions are accepted and implemented locally. ' +
      'Local checks pass; human-audibility, answered/inbound PSTN, ' +
      'interactive browser proofs remain open. The local package passed installation checks. ' +
      '<a href="connect-roadmap.html#gate-3">Roadmap G3-A–G3-E</a> · ' +
      '<a href="connect-gate-3-implementation.html">Numbered implementation</a> · ' +
      '<a href="gate-3-acceptance.json">Acceptance map</a>.';
  }

  const reviewHeading = document.querySelector('#review h2');
  if (reviewHeading) {
    reviewHeading.textContent = 'Finish the required live proofs.';
  }

  const reviewParagraphs = document.querySelectorAll('#review > p');
  if (reviewParagraphs[0]) {
    reviewParagraphs[0].textContent =
      'The contracts and local implementation are in place. Review the ' +
      'recorded local evidence, then close the remaining ' +
      'human-audibility, answered/inbound PSTN and interactive browser checks ' +
      'without weakening the criteria.';
  }
  if (reviewParagraphs[1]) {
    reviewParagraphs[1].textContent =
      'Implementers must keep the numbered HTML plan and acceptance evidence ' +
      'current with every change. The product plan owns behavior, the ' +
      'implementation plan owns task status, and the acceptance map owns ' +
      'traceability.';
  }

  const footer = document.querySelector('footer p');
  if (footer) {
    footer.textContent =
      'Connect · a Rudeless project · Gate 3 planning revision 01 · Local ' +
      'qualification and installed candidate passed; required live proofs remain open.';
  }
}

document.addEventListener('DOMContentLoaded', () => {
  updateDocumentationIndex();
  updateGate3Plan();
});
