const fetch = require('node-fetch');

async function findOut(req, res) {
  if (!req.body.selection || !req.body.selection.length) {
    res.status(400).send('No selection provided');
    return;
  }

  const { selection, mediaType } = req.body;

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

  switch (mediaType) {
    case 'film':
      return findOutFilm(selection, res);
    case 'actor':
      return findOutActor(selection, res);
    default:
      return res.status(400).send('Invalid media type');
  }
}

async function findOutActor(selection, res) {
  const credits = await Promise.all(selection.map((s) => getCredits(s)));
  const films = [];

  credits.forEach((credit) => {
    credit.films.forEach((f) => {
      const film = {
        id: f.id,
        name: f.name,
        imgUrl: f.imgUrl,
        releaseYear: f.releaseYear,
        mediaType: f.mediaType,
      };

      if (!films.some((f) => f.id === film.id)) {
        films.push(film);
      }
    });
  });

  if (films.length === 0) {
    res.status(200).json([]);
    return;
  }

  const results = [];

  films.forEach((film) => {
    const matchingActors = credits.filter((actor) => actor.films.some((f) => f.id === film.id));
    if (matchingActors.length > 1) {
      results.push({
        id: film.id,
        name: film.name,
        imgUrl: film.imgUrl,
        releaseYear: film.releaseYear,
        mediaType: film.mediaType,
        actors: matchingActors.map((actor) => ({
          id: actor.id,
          character: actor.films.find((f) => f.id === film.id).character,
        })),
      })
    }
  });

  return res.json(results);
}

async function findOutFilm(selection, res) {
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

  return res.json(results);
}

async function getCredits({ id, mediaType }) {
  const api = getCreditsApi(mediaType);
  const url = `https://api.themoviedb.org/3/${mediaType}/${id}/${api}?language=en-US`;
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

  if (mediaType === 'person') {
    return {
      id,
      films: data.cast.map((c) => ({
        id: c.id,
        name: c.title || c.name,
        mediaType: c.media_type,
        releaseYear: new Date(c.release_date || c.first_air_date).getFullYear(),
        character: c.character || 'Self',
        imgUrl: c.poster_path ? `https://image.tmdb.org/t/p/w154/${c.poster_path}` : 'https://same.actor/poster.png',
      })),
    }
  }

  return {
    id,
    cast: data.cast.map((c) => ({
      id: c.id,
      name: c.name,
      character: mediaType === 'tv' ? c.roles.map((r) => r.character).join(', ') : c.character,
      imgUrl: c.profile_path ? `https://image.tmdb.org/t/p/w154/${c.profile_path}` : 'https://same.actor/avatar.png',
    })),
  };
}

function getCreditsApi(mediaType) {
  switch (mediaType) {
    case 'person':
      return 'combined_credits';
    case 'tv':
      return 'aggregate_credits';
    default:
      return 'credits';
  }
}

module.exports = { findOut };
