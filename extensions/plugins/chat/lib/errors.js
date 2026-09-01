// Route error helpers — return the plugin route error envelope
// ({__plugin_error, __status, __message}) that the host turns into an HTTP
// error response.

export function fail(status, msg) {
    return { __plugin_error: true, __status: status, __message: msg };
}

export function notFound(msg) {
    return fail(404, msg ?? 'not found');
}

export function invalid(msg) {
    return fail(400, msg ?? 'invalid request');
}
