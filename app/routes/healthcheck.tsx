// learn more: https://fly.io/docs/reference/configuration/#services-http_checks
import type { LoaderArgs } from '@remix-run/node';

export async function loader({ request }: LoaderArgs) {
  try {
    return new Response('OK');
  } catch (error: unknown) {
    console.log('healthcheck ❌', { error });
    return new Response('ERROR', { status: 500 });
  }
}
