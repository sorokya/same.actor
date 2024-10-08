export async function search({ term, mediaType }) {
  if (!term || !term.trim()) {
    return;
  }

  const response = await fetch(`/api/search?mediaType=${encodeURIComponent(mediaType)}&term=${encodeURIComponent(term.substring(0, 100).trim())}`);

  if (response.status === 429) {
    return null;
  }

  return response.json();
}
