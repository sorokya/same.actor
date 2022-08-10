import { Form } from '@remix-run/react';

export default function Search () {
    return (
        <Form className="w-11/12 m-auto">
            <input type="text" placeholder="Walter White" className="w-full p-2 rounded-md text-md border border-gray-300 focus:outline-none focus:ring-1 focus:ring-purple-400 focus:border-transparent" />
            <button type="submit" className="w-full p-2 rounded-md bg-purple-500 text-white text-md hover:bg-purple-600 font-bold mt-4">
                Search
            </button>
        </Form>
    )
}