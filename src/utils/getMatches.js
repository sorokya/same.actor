export async function getMatches({ selection }) {
  const response = await fetch('/api/findOut', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      selection: selection.map((item) => ({
        id: item.id,
        mediaType: item.mediaType,
      })),
    }),
  });

  return response.json();
}
