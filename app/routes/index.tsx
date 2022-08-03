import { Form } from "@remix-run/react";
import { Button } from '../components/Button';


export default function Index() {
  return (
    <main className="relative min-h-screen">
      <Form className="mt-8 space-y-6 max-w-2xl mx-auto bg-white rounded px-3 pb-6 pt-2">
        <h1 className="text-center text-3xl font-bold mt-px">Same actor? 🎬 🤔</h1>
        <h2 className="text-center text-xl font-bold">Let's find out!</h2>
        <div className="rounded-md shadow-sm -space-y-px">
          <div>
            <label htmlFor="search" className="sr-only">Search</label>
            <input id="search" name="search" type="text" required className="appearance-none rounded-none relative block w-full px-3 py-2 border border-gray-300 placeholder-gray-500 text-gray-900 rounded-t-md focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 focus:z-10 sm:text-sm" placeholder="Walter White" />
          </div>
        </div>
        <div>
          <Button type="submit">
            Search
          </Button>
        </div>
      </Form>
    </main>
  );
}
