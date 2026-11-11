// SPDX-License-Identifier: AGPL-3.0-only
// A failed or unconfigured sample service leaves the bundled sample available.
const container = document.querySelector('#more-samples');
if (container) {
  try {
    const response = await fetch('/api/samples', { headers: { Accept: 'application/json' } });
    if (response.ok) {
      const samples = await response.json();
      if (Array.isArray(samples)) for (const sample of samples.slice(0, 100)) {
        if (typeof sample.id !== 'string' || !/^[a-f0-9-]{36}$/.test(sample.id) ||
            typeof sample.name !== 'string' || !/^[a-zA-Z0-9_-]{1,80}\.(?:docx|xlsx|pptx)$/.test(sample.name) ||
            typeof sample.title !== 'string' || sample.title.length > 100) continue;
        const card = document.createElement('a');
        const file = `/api/samples/${sample.id}/${sample.name}`;
        card.href = `./editor.html?open=${encodeURIComponent(file)}`;
        card.className = 'more-sample';
        const title = document.createElement('strong'); title.textContent = sample.title;
        const label = document.createElement('span'); label.textContent = `${sample.name.split('.').pop().toUpperCase()}  ↗`;
        card.append(title, label); container.append(card);
      }
    }
  } catch { /* The static sample works without the sample service. */ }
}
