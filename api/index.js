require('dotenv').config();

const express = require('express');
const bodyParser = require('body-parser');
const app = express();

app.use(bodyParser.json());

const { search, findOut } = require('./controllers');

app.get('/api/search', search);
app.post('/api/findOut', findOut);

app.listen(3001);

console.log('Same.Actor API Listening at http://localhost:3001');
