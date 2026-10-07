// Generated from the Supabase "Novel Platform" schema (supabase gen types typescript).
// Regenerate after migrations change the public schema.
export type Json =
  | string
  | number
  | boolean
  | null
  | { [key: string]: Json | undefined }
  | Json[]

type TranslationSource = Database["public"]["Enums"]["translation_source"]
type TranslationStyle = Database["public"]["Enums"]["translation_style"]
type JobStatus = Database["public"]["Enums"]["translation_job_status"]

type TranslationJob = {
  apply_glossary: boolean
  attempts: number
  chapter_id: number
  created_at: string
  error: string | null
  finished_at: string | null
  id: string
  model: string | null
  priority: number
  requested_by: string | null
  result_id: number | null
  source_lang: string | null
  started_at: string | null
  status: JobStatus
  style: TranslationStyle
  target_lang: string
  updated_at: string
}

export type Database = {
  __InternalSupabase: {
    PostgrestVersion: "14.5"
  }
  public: {
    Tables: {
      authors: {
        Row: { created_at: string; id: number; name: string; slug: string }
        Insert: { created_at?: string; id?: never; name: string; slug: string }
        Update: { created_at?: string; id?: never; name?: string; slug?: string }
        Relationships: []
      }
      categories: {
        Row: { name: string; slug: string; sort_order: number }
        Insert: { name: string; slug: string; sort_order?: number }
        Update: { name?: string; slug?: string; sort_order?: number }
        Relationships: []
      }
      category_translations: {
        Row: { category_slug: string; lang: string; name: string }
        Insert: { category_slug: string; lang: string; name: string }
        Update: { category_slug?: string; lang?: string; name?: string }
        Relationships: [
          { foreignKeyName: "category_translations_category_slug_fkey"; columns: ["category_slug"]; isOneToOne: false; referencedRelation: "categories"; referencedColumns: ["slug"] },
        ]
      }
      chapter_translations: {
        Row: {
          chapter_id: number
          content: string
          created_at: string
          id: number
          lang: string
          model: string | null
          source: TranslationSource
          state: Database["public"]["Enums"]["publish_state"]
          style: TranslationStyle | null
          title: string
          updated_at: string
          word_count: number | null
        }
        Insert: {
          chapter_id: number
          content: string
          created_at?: string
          id?: never
          lang: string
          model?: string | null
          source: TranslationSource
          state?: Database["public"]["Enums"]["publish_state"]
          style?: TranslationStyle | null
          title: string
          updated_at?: string
          word_count?: number | null
        }
        Update: {
          chapter_id?: number
          content?: string
          created_at?: string
          id?: never
          lang?: string
          model?: string | null
          source?: TranslationSource
          state?: Database["public"]["Enums"]["publish_state"]
          style?: TranslationStyle | null
          title?: string
          updated_at?: string
          word_count?: number | null
        }
        Relationships: [
          { foreignKeyName: "chapter_translations_chapter_id_fkey"; columns: ["chapter_id"]; isOneToOne: false; referencedRelation: "chapters"; referencedColumns: ["id"] },
          { foreignKeyName: "chapter_translations_lang_fkey"; columns: ["lang"]; isOneToOne: false; referencedRelation: "languages"; referencedColumns: ["code"] },
        ]
      }
      chapters: {
        Row: { created_at: string; id: number; novel_id: number; number: number; published_at: string | null; updated_at: string }
        Insert: { created_at?: string; id?: never; novel_id: number; number: number; published_at?: string | null; updated_at?: string }
        Update: { created_at?: string; id?: never; novel_id?: number; number?: number; published_at?: string | null; updated_at?: string }
        Relationships: [
          { foreignKeyName: "chapters_novel_id_fkey"; columns: ["novel_id"]; isOneToOne: false; referencedRelation: "novels"; referencedColumns: ["id"] },
        ]
      }
      collection_items: {
        Row: { collection_key: string; novel_id: number; position: number }
        Insert: { collection_key: string; novel_id: number; position?: number }
        Update: { collection_key?: string; novel_id?: number; position?: number }
        Relationships: [
          { foreignKeyName: "collection_items_collection_key_fkey"; columns: ["collection_key"]; isOneToOne: false; referencedRelation: "collections"; referencedColumns: ["key"] },
          { foreignKeyName: "collection_items_novel_id_fkey"; columns: ["novel_id"]; isOneToOne: false; referencedRelation: "novels"; referencedColumns: ["id"] },
        ]
      }
      collection_translations: {
        Row: { collection_key: string; kicker: string; lang: string; title: string }
        Insert: { collection_key: string; kicker?: string; lang: string; title: string }
        Update: { collection_key?: string; kicker?: string; lang?: string; title?: string }
        Relationships: [
          { foreignKeyName: "collection_translations_collection_key_fkey"; columns: ["collection_key"]; isOneToOne: false; referencedRelation: "collections"; referencedColumns: ["key"] },
        ]
      }
      collections: {
        Row: { key: string; name: string; sort_order: number }
        Insert: { key: string; name: string; sort_order?: number }
        Update: { key?: string; name?: string; sort_order?: number }
        Relationships: []
      }
      favorites: {
        Row: { created_at: string; novel_id: number; user_id: string }
        Insert: { created_at?: string; novel_id: number; user_id?: string }
        Update: { created_at?: string; novel_id?: number; user_id?: string }
        Relationships: [
          { foreignKeyName: "favorites_novel_id_fkey"; columns: ["novel_id"]; isOneToOne: false; referencedRelation: "novels"; referencedColumns: ["id"] },
        ]
      }
      glossary_entries: {
        Row: { created_at: string; id: number; lang: string; note: string | null; novel_id: number; term_key: string; updated_at: string; value: string }
        Insert: { created_at?: string; id?: never; lang: string; note?: string | null; novel_id: number; term_key: string; updated_at?: string; value: string }
        Update: { created_at?: string; id?: never; lang?: string; note?: string | null; novel_id?: number; term_key?: string; updated_at?: string; value?: string }
        Relationships: [
          { foreignKeyName: "glossary_entries_lang_fkey"; columns: ["lang"]; isOneToOne: false; referencedRelation: "languages"; referencedColumns: ["code"] },
          { foreignKeyName: "glossary_entries_novel_id_fkey"; columns: ["novel_id"]; isOneToOne: false; referencedRelation: "novels"; referencedColumns: ["id"] },
        ]
      }
      languages: {
        Row: { code: string; enabled: boolean; name: string; native_name: string; sort_order: number }
        Insert: { code: string; enabled?: boolean; name: string; native_name: string; sort_order?: number }
        Update: { code?: string; enabled?: boolean; name?: string; native_name?: string; sort_order?: number }
        Relationships: []
      }
      novel_translations: {
        Row: { description: string; lang: string; novel_id: number; source: TranslationSource; title: string; updated_at: string }
        Insert: { description?: string; lang: string; novel_id: number; source?: TranslationSource; title: string; updated_at?: string }
        Update: { description?: string; lang?: string; novel_id?: number; source?: TranslationSource; title?: string; updated_at?: string }
        Relationships: [
          { foreignKeyName: "novel_translations_lang_fkey"; columns: ["lang"]; isOneToOne: false; referencedRelation: "languages"; referencedColumns: ["code"] },
          { foreignKeyName: "novel_translations_novel_id_fkey"; columns: ["novel_id"]; isOneToOne: false; referencedRelation: "novels"; referencedColumns: ["id"] },
        ]
      }
      novels: {
        Row: {
          author_id: number
          category_slug: string
          chapter_count: number
          cover_palette: Json | null
          cover_url: string | null
          created_at: string
          id: number
          is_published: boolean
          original_lang: string
          published_year: number | null
          rating_avg: number
          rating_count: number
          read_count: number
          slug: string
          status: Database["public"]["Enums"]["novel_status"]
          updated_at: string
        }
        Insert: {
          author_id: number
          category_slug: string
          chapter_count?: number
          cover_palette?: Json | null
          cover_url?: string | null
          created_at?: string
          id?: never
          is_published?: boolean
          original_lang: string
          published_year?: number | null
          rating_avg?: number
          rating_count?: number
          read_count?: number
          slug: string
          status?: Database["public"]["Enums"]["novel_status"]
          updated_at?: string
        }
        Update: {
          author_id?: number
          category_slug?: string
          chapter_count?: number
          cover_palette?: Json | null
          cover_url?: string | null
          created_at?: string
          id?: never
          is_published?: boolean
          original_lang?: string
          published_year?: number | null
          rating_avg?: number
          rating_count?: number
          read_count?: number
          slug?: string
          status?: Database["public"]["Enums"]["novel_status"]
          updated_at?: string
        }
        Relationships: [
          { foreignKeyName: "novels_author_id_fkey"; columns: ["author_id"]; isOneToOne: false; referencedRelation: "authors"; referencedColumns: ["id"] },
          { foreignKeyName: "novels_category_slug_fkey"; columns: ["category_slug"]; isOneToOne: false; referencedRelation: "categories"; referencedColumns: ["slug"] },
          { foreignKeyName: "novels_original_lang_fkey"; columns: ["original_lang"]; isOneToOne: false; referencedRelation: "languages"; referencedColumns: ["code"] },
        ]
      }
      profiles: {
        Row: { avatar_url: string | null; created_at: string; default_content_lang: string | null; display_name: string | null; id: string; reader_prefs: Json; ui_lang: string; updated_at: string }
        Insert: { avatar_url?: string | null; created_at?: string; default_content_lang?: string | null; display_name?: string | null; id: string; reader_prefs?: Json; ui_lang?: string; updated_at?: string }
        Update: { avatar_url?: string | null; created_at?: string; default_content_lang?: string | null; display_name?: string | null; id?: string; reader_prefs?: Json; ui_lang?: string; updated_at?: string }
        Relationships: [
          { foreignKeyName: "profiles_default_content_lang_fkey"; columns: ["default_content_lang"]; isOneToOne: false; referencedRelation: "languages"; referencedColumns: ["code"] },
        ]
      }
      reading_history: {
        Row: { chapter_id: number; chapter_number: number; lang: string; last_read_at: string; novel_id: number; progress: number; user_id: string }
        Insert: { chapter_id: number; chapter_number: number; lang: string; last_read_at?: string; novel_id: number; progress?: number; user_id?: string }
        Update: { chapter_id?: number; chapter_number?: number; lang?: string; last_read_at?: string; novel_id?: number; progress?: number; user_id?: string }
        Relationships: [
          { foreignKeyName: "reading_history_chapter_id_fkey"; columns: ["chapter_id"]; isOneToOne: false; referencedRelation: "chapters"; referencedColumns: ["id"] },
          { foreignKeyName: "reading_history_lang_fkey"; columns: ["lang"]; isOneToOne: false; referencedRelation: "languages"; referencedColumns: ["code"] },
          { foreignKeyName: "reading_history_novel_id_fkey"; columns: ["novel_id"]; isOneToOne: false; referencedRelation: "novels"; referencedColumns: ["id"] },
        ]
      }
      translation_jobs: {
        Row: TranslationJob
        Insert: Partial<TranslationJob> & { chapter_id: number; target_lang: string }
        Update: Partial<TranslationJob>
        Relationships: [
          { foreignKeyName: "translation_jobs_chapter_id_fkey"; columns: ["chapter_id"]; isOneToOne: false; referencedRelation: "chapters"; referencedColumns: ["id"] },
          { foreignKeyName: "translation_jobs_result_id_fkey"; columns: ["result_id"]; isOneToOne: false; referencedRelation: "chapter_translations"; referencedColumns: ["id"] },
          { foreignKeyName: "translation_jobs_source_lang_fkey"; columns: ["source_lang"]; isOneToOne: false; referencedRelation: "languages"; referencedColumns: ["code"] },
          { foreignKeyName: "translation_jobs_target_lang_fkey"; columns: ["target_lang"]; isOneToOne: false; referencedRelation: "languages"; referencedColumns: ["code"] },
        ]
      }
    }
    Views: {
      [_ in never]: never
    }
    Functions: {
      claim_translation_job: {
        Args: { worker_model?: string }
        Returns: TranslationJob
        SetofOptions: { from: "*"; to: "translation_jobs"; isOneToOne: true; isSetofReturn: false }
      }
      request_translation: {
        Args: { p_apply_glossary?: boolean; p_chapter_id: number; p_style?: TranslationStyle; p_target_lang: string }
        Returns: TranslationJob
        SetofOptions: { from: "*"; to: "translation_jobs"; isOneToOne: true; isSetofReturn: false }
      }
    }
    Enums: {
      novel_status: "ongoing" | "completed"
      publish_state: "draft" | "published"
      translation_job_status: "queued" | "running" | "succeeded" | "failed" | "cancelled"
      translation_source: "original" | "human" | "ai"
      translation_style: "literal" | "natural" | "literary" | "casual"
    }
    CompositeTypes: {
      [_ in never]: never
    }
  }
}
