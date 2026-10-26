import { ZodError } from 'zod';
import { HttpError } from '../lib/errors.js';

const PG_MESSAGES = {
  '23505': [409, 'A record with that value already exists'],
  '23503': [400, 'A referenced record does not exist'],
  '23514': [400, 'A value is not allowed'],
  '22P02': [400, 'Invalid identifier'],
};

// eslint-disable-next-line no-unused-vars
export function errorHandler(err, req, res, _next) {
  if (err instanceof ZodError) {
    return res.status(400).json({
      error: 'Invalid request',
      details: err.issues.map((i) => ({ path: i.path.join('.'), message: i.message })),
    });
  }
  if (err instanceof HttpError) {
    return res.status(err.status).json({ error: err.message, ...(err.details && { details: err.details }) });
  }
  if (err.type === 'entity.parse.failed') return res.status(400).json({ error: 'Malformed JSON body' });
  if (err.code && PG_MESSAGES[err.code]) {
    const [status, message] = PG_MESSAGES[err.code];
    return res.status(status).json({ error: message, ...(err.constraint && { constraint: err.constraint }) });
  }
  console.error(err);
  res.status(500).json({ error: 'Something went wrong' });
}
