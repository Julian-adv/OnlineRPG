import { writable } from 'svelte/store'

export interface LandAccount {
  merchant_player_id: number
  treasury: number
  plots: number
  monthly_tax: number
  next_tax: number
  next_due: { year: number; month: number; day: number }
  due_in_seconds: number
  missed: number
  recovery_cost: number
  free_months: number
  error: string | null
}

export const landAccount = writable<LandAccount | null>(null)
export const landAccountError = writable<string | null>(null)
export const landTransferPending = writable(false)
