const fetch = require('node-fetch');

async function findOut(req, res) {
  if (!req.body.selection || !req.body.selection.length) {
    res.status(400).send('No selection provided');
    return;
  }

  const { selection } = req.body;

  if (selection.length > 5) {
    res.status(400).send('Too many selections');
    return;
  }

  if (
    selection.some((s) => s.mediaType === 'person') &&
    selection.some((s) => ['tv', 'movie'].includes(s.mediaType))
  ) {
    res.status(400).send('Can not select a person and a media');
    return;
  }

  const credits = await Promise.all(selection.map((s) => getCredits(s)));

  const cast = [];

  credits.forEach((credit) => {
    credit.cast.forEach((c) => {
      const castMember = {
        id: c.id,
        name: c.name,
        imgUrl: c.imgUrl,
      };

      if (!cast.some((c) => c.id === castMember.id)) {
        cast.push(castMember);
      }
    });
  });

  if (cast.length === 0) {
    res.status(200).json([]);
    return;
  }

  const results = [];

  cast.forEach((member) => {
    const matchingMedia = credits.filter((item) => item.cast.some((c) => c.id === member.id));
    if (matchingMedia.length > 1) {
      results.push({
        id: member.id,
        name: member.name,
        imgUrl: member.imgUrl,
        media: matchingMedia.map((m) => ({
          id: m.id,
          character: m.cast.find((c) => c.id === member.id).character,
        })),
      })
    }
  });

  res.json(results);
}

async function getCredits({ id, mediaType }) {
  const url = `https://api.themoviedb.org/3/${mediaType}/${id}/${mediaType === 'tv' ? 'aggregate_' : ''}credits?language=en-US`;
  const options = {
    method: 'GET',
    headers: {
      accept: 'application/json',
      Authorization: `Bearer ${process.env.TMDB_KEY}`,
    },
  };

  const response = await fetch(url, options);

  if (!response.ok) {
    console.error('Response not OK from the movie db', response);
    return null;
  }

  const data = await response.json();
  if (!data.cast) {
    return null;
  }

  return {
    id,
    mediaType,
    cast: data.cast.map((c) => ({
      id: c.id,
      name: c.name,
      character: mediaType === 'tv' ? c.roles.map((r) => r.character).join(', ') : c.character,
      imgUrl: `https://image.tmdb.org/t/p/w154/${c.profile_path}`,
    })),
  };
}

module.exports = { findOut };
