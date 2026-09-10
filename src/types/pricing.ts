export interface RemoteModelPricing {
  input_cost_per_token?: number;
  output_cost_per_token?: number;
  cache_read_input_token_cost?: number;
  cache_creation_input_token_cost?: number;
}

export interface PricingCatalogItem {
  key: string;
  model: string;
  provider: string;
  inputPer1M: number;
  outputPer1M: number;
  cacheWritePer1M: number;
  cacheReadPer1M: number;
}

export interface PricingCatalogStatus {
  model_count: number;
  updated_at: string | null;
}
