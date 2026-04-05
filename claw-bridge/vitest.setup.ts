import { vi } from 'vitest';

// Helper to create a chainable mock for Supabase query builder
function createSupabaseQueryChain() {
  return {
    select: vi.fn(function () {
      return this;
    }),
    eq: vi.fn(function () {
      return this;
    }),
    gte: vi.fn(function () {
      return this;
    }),
    lte: vi.fn(function () {
      return this;
    }),
    single: vi.fn(async function () {
      return { data: null, error: null };
    }),
  };
}

// Mock the ClawNetwork SDK since it's not fully available in test environment
vi.mock('@clawlabz/clawnetwork-sdk', () => ({
  ClawClient: vi.fn(function () {
    this.transfer = vi.fn();
    this.getTransaction = vi.fn();
    this.getBalance = vi.fn();
  }),
  Wallet: {
    fromPrivateKey: vi.fn(function (key: string) {
      return {
        address: '0x' + 'f'.repeat(64),
        privateKey: key,
      };
    }),
  },
  toHex: vi.fn((val: any) => '0x' + val.toString(16)),
}));

// Mock Supabase since it requires connection to a real instance
vi.mock('@supabase/supabase-js', () => ({
  createClient: vi.fn(() => ({
    rpc: vi.fn(async () => ({ data: null, error: null })),
    from: vi.fn(() => createSupabaseQueryChain()),
  })),
}));
