const { rateLimit } = require('express-rate-limit');

const limiter = rateLimit({
  windowMs: 1000,
  limit: 5,
  standardHeaders: 'draft-7',
  legacyHeaders: false,
});

require('dotenv').config();

const express = require('express');
const bodyParser = require('body-parser');
const app = express();

app.use(limiter);
app.use(bodyParser.json());

const { search, findOut } = require('./controllers');

app.get('/api/search', search);
app.post('/api/findOut', findOut);

app.listen(3001);

console.log('Same.Actor API Listening at http://localhost:3001');
