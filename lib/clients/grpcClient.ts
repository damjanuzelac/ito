import { ItoService, TranscribeStreamRequest } from '@/app/generated/ito_pb'
import { createClient } from '@connectrpc/connect'
import { createConnectTransport } from '@connectrpc/connect-node'

class GrpcClient {
  private client: ReturnType<typeof createClient<typeof ItoService>>

  constructor() {
    const transport = createConnectTransport({
      baseUrl: import.meta.env.VITE_GRPC_BASE_URL,
      httpVersion: '1.1',
    })
    console.log(
      'Creating gRPC client with base URL:',
      import.meta.env.VITE_GRPC_BASE_URL,
    )
    this.client = createClient(ItoService, transport)
  }

  async transcribeStreamV2(
    stream: AsyncIterable<TranscribeStreamRequest>,
    signal?: AbortSignal,
  ) {
    return this.client.transcribeStreamV2(stream, { signal })
  }
}

export const grpcClient = new GrpcClient()
