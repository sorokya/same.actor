export async function getMatches({ selection, mediaType }) {
  const response = await fetch('/api/findOut', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      mediaType,
      selection: selection.map((item) => ({
        id: item.id,
        mediaType: item.mediaType,
      })),
    }),
  });

  if (response.status === 429) {
    return null;
  }

  return response.json();
}
