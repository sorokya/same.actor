const fetch = require('node-fetch');

const MEDIA_TYPES = ['tv', 'movie', 'person'];

async function search(req, res) {
  const term = req.query.term;
  if (!term) {
    return res.status(400).send('Search term is required');
  }

  if (term.length < 3) {
    return res
      .status(400)
      .send('Search term must be at least 3 characters in length');
  }

  const url = `https://api.themoviedb.org/3/search/multi?query=${encodeURIComponent(
    term,
  )}&include_adult=false&language=en-US&page=1`;
  const options = {
    method: 'GET',
    headers: {
      accept: 'application/json',
      Authorization: `Bearer ${process.env.TMDB_KEY}`,
    },
  };

  try {
    const response = await fetch(url, options);
    if (!response.ok) {
      console.error('Response not OK from the movie db', response);
      return res.status(500).send('There was an error processing your search');
    }

    const data = await response.json();
    return res.json(
      data.results
        .filter(
          (d) =>
            (d.poster_path || d.profile_path) &&
            (d.first_air_date || d.release_date || d.media_type === 'person') &&
            MEDIA_TYPES.includes(d.media_type),
        )
        .map((d) => ({
          id: d.id,
          mediaType: d.media_type,
          name: d.name || d.title,
          overview:
            d.media_type === 'person'
              ? `Known for: ${d.known_for
                .map(
                  (i) =>
                    `${i.name || i.title} (${new Date(
                      i.release_date || i.first_air_date,
                    ).getFullYear()})`,
                )
                .join(', ')}`
              : d.overview,
          imgUrl: `https://image.tmdb.org/t/p/w154/${d.poster_path || d.profile_path
            }`,
          releaseYear:
            d.media_type === 'person'
              ? null
              : new Date(d.release_date || d.first_air_date).getFullYear(),
        })),
    );
  } catch (err) {
    console.error('Failed to search the movie db', err);
    return res.status(500).send('There was an error processing your search');
  }
}

module.exports = { search };
