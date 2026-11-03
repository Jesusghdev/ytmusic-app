export interface PlayerState {
  title: string;
  artist: string;
  album: string;
  artwork: string;
  video_id: string;
  duration: number;
  position: number;
  playing: boolean;
  volume: number;
  liked: boolean;
  updated_at: number;
  next_track?: UpcomingTrack | null;
}

export interface UpcomingTrack {
  title: string;
  artist: string;
  album: string;
  duration: number;
}

export interface LyricLine {
  time: number;
  text: string;
}

export interface Lyrics {
  lines: LyricLine[];
  synced: boolean;
  instrumental: boolean;
  source: string;
  message: string;
}

export interface RemoteInfo {
  url: string;
  port: number;
}
