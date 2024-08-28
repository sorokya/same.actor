export async function search({ term }) {
  if (!term || !term.trim()) {
    return;
  }

  const response = await fetch(`/api/search?term=${encodeURIComponent(term.substring(0, 100).trim())}`);
  return await response.json();
}
