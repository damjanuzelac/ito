import { fastify } from 'fastify'
import { fastifyConnectPlugin } from '@connectrpc/connect-fastify'
import itoServiceRoutes from './services/ito/itoService.js'
import { errorInterceptor } from './services/errorInterceptor.js'
import { loggingInterceptor } from './services/loggingInterceptor.js'
import { createValidationInterceptor } from './services/validationInterceptor.js'
import dotenv from 'dotenv'
import cors from '@fastify/cors'

dotenv.config()

// Create the main server function
export const startServer = async () => {
  const connectRpcServer = fastify({
    logger: process.env.SHOW_ALL_REQUEST_LOGS === 'true',
  })

  await connectRpcServer.register(cors, { origin: '*' })

  // Register the Connect RPC plugin with our service routes and interceptors
  await connectRpcServer.register(fastifyConnectPlugin, {
    routes: router => {
      itoServiceRoutes(router)
    },
    // Order matters: logging -> validation -> error handling
    interceptors: [
      loggingInterceptor,
      createValidationInterceptor(),
      errorInterceptor,
    ],
  })

  // Error handling - this handles Fastify-level errors, not RPC errors
  connectRpcServer.setErrorHandler((error, _, reply) => {
    connectRpcServer.log.error(error)
    reply.status(500).send({
      error: 'Internal Server Error',
      message: error.message,
    })
  })

  // Basic REST routes for health checks
  connectRpcServer.get('/', async (_, reply) => {
    reply.type('text/plain')
    reply.send('Welcome to the Ito Connect RPC server!')
  })

  connectRpcServer.get('/health', async (_, reply) => {
    reply.send({ status: 'ok' })
  })

  // Start the server
  const rpcPort = 3000
  const host = '0.0.0.0'

  try {
    await connectRpcServer.listen({ port: rpcPort, host })
    console.log(`🚀 Connect RPC server listening on ${host}:${rpcPort}`)
  } catch (err) {
    connectRpcServer.log.error(err)
    process.exit(1)
  }
}
