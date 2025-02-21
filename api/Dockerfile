FROM node:22

WORKDIR /api

COPY package*.json ./

RUN npm install --omit=dev

COPY . .

EXPOSE 3001

CMD ["node", "index.js"]
