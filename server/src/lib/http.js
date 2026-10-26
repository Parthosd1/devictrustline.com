// Lets async route handlers pass rejections to the Express error handler.
export const route = (fn) => (req, res, next) => Promise.resolve(fn(req, res, next)).catch(next);
