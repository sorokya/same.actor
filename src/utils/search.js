export async function search({ term }) {
  const response = await fetch(`/api/search?term=${encodeURIComponent(term)}`);
  return await response.json();
}
